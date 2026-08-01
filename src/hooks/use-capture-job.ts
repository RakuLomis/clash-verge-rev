import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useCallback, useEffect, useMemo, useState } from 'react'

import { trafficTracerCaptureLockKey } from '@/hooks/use-traffic-tracer-worker'
import {
  cancelTrafficTracerJob,
  getTrafficTracerJob,
  startTrafficTracerCapture,
} from '@/services/cmds'
import type {
  CaptureStartRequest,
  JobProgressEvent,
  JobSnapshot,
} from '@/types/traffic-tracer'

const ACTIVE_JOB_STORAGE_KEY = 'traffictracer.activeJobId'
const JOB_STARTED_STORAGE_KEY = 'traffictracer.activeJobStartedAt'
const TERMINAL_STATES = new Set([
  'completed',
  'failed',
  'cancelled',
  'interrupted',
])
const JOB_EVENTS = [
  'traffictracer://job-progress',
  'traffictracer://job-state',
  'traffictracer://job-completed',
  'traffictracer://job-failed',
  'traffictracer://job-cancelled',
] as const

export const trafficTracerJobKey = (jobId: string) =>
  ['trafficTracer', 'job', jobId] as const

export function mergeTrafficTracerProgress(
  snapshot: JobSnapshot | undefined,
  progress: JobProgressEvent,
): JobSnapshot | undefined {
  if (!snapshot || snapshot.job_id !== progress.job_id) return snapshot
  return {
    ...snapshot,
    state: progress.state,
    stage: progress.stage,
    progress: progress.progress,
    message: progress.message,
  }
}

export function useCaptureJob(initialJobId?: string | null) {
  const queryClient = useQueryClient()
  const [jobId, setJobId] = useState<string | null>(() =>
    initialJobId === undefined
      ? localStorage.getItem(ACTIVE_JOB_STORAGE_KEY)
      : initialJobId,
  )
  const [jobStartedAt, setJobStartedAt] = useState<string | null>(() =>
    jobId ? localStorage.getItem(JOB_STARTED_STORAGE_KEY) : null,
  )
  const [progressEvents, setProgressEvents] = useState<JobProgressEvent[]>([])

  const rememberJob = useCallback((nextJobId: string) => {
    const startedAt = new Date().toISOString()
    localStorage.setItem(ACTIVE_JOB_STORAGE_KEY, nextJobId)
    localStorage.setItem(JOB_STARTED_STORAGE_KEY, startedAt)
    setJobStartedAt(startedAt)
    setProgressEvents([])
    setJobId(nextJobId)
  }, [])

  const clearJob = useCallback(() => {
    localStorage.removeItem(ACTIVE_JOB_STORAGE_KEY)
    localStorage.removeItem(JOB_STARTED_STORAGE_KEY)
    setJobStartedAt(null)
    setProgressEvents([])
    setJobId(null)
  }, [])

  const jobQuery = useQuery({
    queryKey: jobId
      ? trafficTracerJobKey(jobId)
      : ['trafficTracer', 'job', 'none'],
    queryFn: () => getTrafficTracerJob(jobId!),
    enabled: jobId !== null,
    refetchInterval: ({ state }) =>
      state.data && TERMINAL_STATES.has(state.data.state) ? false : 1000,
  })

  useEffect(() => {
    if (!jobId) return

    let disposed = false
    let unlisteners: UnlistenFn[] = []
    const updateSnapshot = (snapshot: JobSnapshot) => {
      if (snapshot.job_id !== jobId) return
      queryClient.setQueryData(trafficTracerJobKey(jobId), snapshot)
      if (TERMINAL_STATES.has(snapshot.state)) {
        localStorage.removeItem(ACTIVE_JOB_STORAGE_KEY)
        localStorage.removeItem(JOB_STARTED_STORAGE_KEY)
        void queryClient.invalidateQueries({
          queryKey: trafficTracerCaptureLockKey,
        })
      }
    }
    const updateProgress = (progress: JobProgressEvent) => {
      if (progress.job_id !== jobId) return
      setProgressEvents((events) => {
        const previous = events.at(-1)
        if (
          previous?.timestamp === progress.timestamp &&
          previous.stage === progress.stage &&
          previous.message === progress.message
        ) {
          return events
        }
        return [...events, progress].slice(-100)
      })
      queryClient.setQueryData<JobSnapshot>(
        trafficTracerJobKey(jobId),
        (snapshot) => mergeTrafficTracerProgress(snapshot, progress),
      )
    }

    Promise.all(
      JOB_EVENTS.map((eventName) =>
        eventName === 'traffictracer://job-progress'
          ? listen<JobProgressEvent>(eventName, ({ payload }) =>
              updateProgress(payload),
            )
          : listen<JobSnapshot>(eventName, ({ payload }) =>
              updateSnapshot(payload),
            ),
      ),
    )
      .then((registered) => {
        if (disposed) {
          registered.forEach((unlisten) => unlisten())
        } else {
          unlisteners = registered
        }
      })
      .catch((error) =>
        console.error('[TrafficTracer] Job event registration failed:', error),
      )

    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => unlisten())
      unlisteners = []
    }
  }, [jobId, queryClient])

  useEffect(() => {
    if (jobQuery.data && TERMINAL_STATES.has(jobQuery.data.state)) {
      localStorage.removeItem(ACTIVE_JOB_STORAGE_KEY)
      localStorage.removeItem(JOB_STARTED_STORAGE_KEY)
    }
  }, [jobQuery.data])

  const startMutation = useMutation({
    mutationFn: (request: CaptureStartRequest) =>
      startTrafficTracerCapture(request),
    onSuccess: (snapshot) => {
      rememberJob(snapshot.job_id)
      queryClient.setQueryData(trafficTracerJobKey(snapshot.job_id), snapshot)
      void queryClient.invalidateQueries({
        queryKey: trafficTracerCaptureLockKey,
      })
    },
  })

  const cancelMutation = useMutation({
    mutationFn: ({ reason }: { reason?: string }) => {
      if (!jobId) throw new Error('No TrafficTracer Job is active')
      return cancelTrafficTracerJob(jobId, reason)
    },
    onMutate: async () => {
      if (!jobId) return undefined
      await queryClient.cancelQueries({ queryKey: trafficTracerJobKey(jobId) })
      const previous = queryClient.getQueryData<JobSnapshot>(
        trafficTracerJobKey(jobId),
      )
      queryClient.setQueryData<JobSnapshot>(
        trafficTracerJobKey(jobId),
        (snapshot) =>
          snapshot ? { ...snapshot, cancel_requested: true } : snapshot,
      )
      return { jobId, previous }
    },
    onError: (_error, _variables, context) => {
      if (context?.previous) {
        queryClient.setQueryData(
          trafficTracerJobKey(context.jobId),
          context.previous,
        )
      }
    },
    onSuccess: (snapshot) => {
      queryClient.setQueryData(trafficTracerJobKey(snapshot.job_id), snapshot)
    },
  })

  const currentProgressEvents = useMemo(
    () => progressEvents.filter((event) => event.job_id === jobId),
    [jobId, progressEvents],
  )

  return {
    jobId,
    job: jobQuery.data,
    jobQuery,
    jobStartedAt,
    progressEvents: currentProgressEvents,
    startCapture: startMutation.mutateAsync,
    startMutation,
    cancelJob: cancelMutation.mutateAsync,
    cancelMutation,
    clearJob,
  }
}
