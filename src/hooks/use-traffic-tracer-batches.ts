import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'

import { trafficTracerCaptureLockKey } from '@/hooks/use-traffic-tracer-worker'
import {
  cancelTrafficTracerBatch,
  getTrafficTracerBatch,
  listTrafficTracerBatches,
  resumeTrafficTracerBatch,
  startTrafficTracerBatch,
} from '@/services/cmds'
import type { BatchStartRequest } from '@/types/traffic-tracer'

const ACTIVE_BATCH_KEY = 'traffictracer.activeBatchId'
const TERMINAL = new Set(['completed', 'failed', 'cancelled', 'interrupted'])

interface ResumeTransition {
  batchId: string
  baselineAttempt: number
}

export const trafficTracerBatchListKey = (workspaceRoot: string) =>
  ['trafficTracer', 'batches', workspaceRoot] as const
export const trafficTracerBatchKey = (batchId: string) =>
  ['trafficTracer', 'batch', batchId] as const

export function useTrafficTracerBatches(workspaceRoot = '', enabled = true) {
  const queryClient = useQueryClient()
  const [batchId, setBatchId] = useState<string | null>(() =>
    localStorage.getItem(ACTIVE_BATCH_KEY),
  )
  const [resumeTransition, setResumeTransition] =
    useState<ResumeTransition | null>(null)
  const listQuery = useQuery({
    queryKey: trafficTracerBatchListKey(workspaceRoot),
    queryFn: listTrafficTracerBatches,
    enabled,
    refetchInterval: false,
    retry: 1,
  })

  const recoveredBatch = batchId
    ? undefined
    : listQuery.data?.batches.find((batch) =>
        ['running', 'failed', 'interrupted'].includes(batch.state),
      )
  const activeBatchId = batchId ?? recoveredBatch?.batch_id ?? null

  useEffect(() => {
    if (recoveredBatch) {
      localStorage.setItem(ACTIVE_BATCH_KEY, recoveredBatch.batch_id)
    }
  }, [recoveredBatch])

  const listedBatch = activeBatchId
    ? listQuery.data?.batches.find((batch) => batch.batch_id === activeBatchId)
    : undefined

  const statusQuery = useQuery({
    queryKey: activeBatchId
      ? trafficTracerBatchKey(activeBatchId)
      : ['trafficTracer', 'batch', 'none'],
    queryFn: () => getTrafficTracerBatch(activeBatchId!),
    enabled: enabled && Boolean(activeBatchId),
    refetchInterval: ({ state }) => {
      const transitionPending =
        resumeTransition?.batchId === activeBatchId &&
        (!state.data ||
          (state.data.batch.state !== 'running' &&
            state.data.batch.resume.attempt <=
              resumeTransition.baselineAttempt))
      if (transitionPending) return 1000
      return state.data && TERMINAL.has(state.data.batch.state) ? false : 1000
    },
  })

  const resumeTransitionActive =
    resumeTransition?.batchId === activeBatchId &&
    (!statusQuery.data ||
      (statusQuery.data.batch.state !== 'running' &&
        statusQuery.data.batch.resume.attempt <=
          resumeTransition.baselineAttempt))

  useEffect(() => {
    const state = statusQuery.data?.batch.state
    if (state && TERMINAL.has(state) && !resumeTransitionActive) {
      localStorage.removeItem(ACTIVE_BATCH_KEY)
    }
  }, [resumeTransitionActive, statusQuery.data?.batch.state])

  const liveStatus = statusQuery.data
  const batchStatus =
    listedBatch &&
    (!liveStatus ||
      Date.parse(listedBatch.updated_at) >=
        Date.parse(liveStatus.batch.updated_at))
      ? { batch: listedBatch, job: liveStatus?.job ?? null }
      : liveStatus

  const remember = (id: string) => {
    localStorage.setItem(ACTIVE_BATCH_KEY, id)
    setBatchId(id)
  }
  const invalidate = () => {
    void queryClient.invalidateQueries({
      queryKey: ['trafficTracer', 'batches'],
    })
    void queryClient.invalidateQueries({
      queryKey: trafficTracerCaptureLockKey,
    })
  }
  const startMutation = useMutation({
    mutationFn: (request: BatchStartRequest) =>
      startTrafficTracerBatch(request),
    onSuccess: (job) => {
      remember(job.job_id)
      invalidate()
    },
  })
  const cancelMutation = useMutation({
    mutationFn: (reason?: string) => {
      if (!activeBatchId) throw new Error('No TrafficTracer batch is selected')
      return cancelTrafficTracerBatch(activeBatchId, reason)
    },
    onSuccess: (status) => {
      queryClient.setQueryData(
        trafficTracerBatchKey(status.batch.batch_id),
        status,
      )
      invalidate()
    },
  })
  const resumeMutation = useMutation({
    mutationFn: () => {
      if (!activeBatchId) throw new Error('No TrafficTracer batch is selected')
      return resumeTrafficTracerBatch(activeBatchId)
    },
    onMutate: () => {
      if (!activeBatchId) return
      remember(activeBatchId)
      setResumeTransition({
        batchId: activeBatchId,
        baselineAttempt: batchStatus?.batch.resume.attempt ?? 0,
      })
    },
    onSuccess: (job) => {
      remember(job.job_id)
      void queryClient.invalidateQueries({
        queryKey: trafficTracerBatchKey(job.job_id),
      })
      invalidate()
    },
    onError: () => setResumeTransition(null),
  })

  return {
    batchId: activeBatchId,
    batchStatus,
    batches: listQuery.data?.batches ?? [],
    corruptBatches: listQuery.data?.corrupt ?? [],
    listQuery,
    statusQuery,
    startBatch: startMutation.mutateAsync,
    startMutation,
    cancelBatch: cancelMutation.mutateAsync,
    cancelMutation,
    resumeBatch: resumeMutation.mutateAsync,
    resumeMutation,
    resuming: resumeMutation.isPending || resumeTransitionActive,
    selectBatch: remember,
  }
}
