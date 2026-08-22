import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect, useState } from 'react'

import { trafficTracerCaptureLockKey } from '@/hooks/use-traffic-tracer-worker'
import {
  cancelTrafficTracerBatch,
  interruptTrafficTracerBatch,
  getTrafficTracerBatch,
  listTrafficTracerBatches,
  resumeTrafficTracerBatch,
  startTrafficTracerBatch,
} from '@/services/cmds'
import type { BatchStartRequest } from '@/types/traffic-tracer'

const ACTIVE_BATCH_KEY = 'traffictracer.activeBatchId'
const VIEWED_BATCH_KEY = 'traffictracer.viewedBatchId'

const scopedKey = (prefix: string, workspaceRoot: string) =>
  workspaceRoot.trim()
    ? `${prefix}:${encodeURIComponent(workspaceRoot)}`
    : prefix
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
  const activeStorageKey = scopedKey(ACTIVE_BATCH_KEY, workspaceRoot)
  const viewedStorageKey = scopedKey(VIEWED_BATCH_KEY, workspaceRoot)
  const [batchId, setBatchId] = useState<string | null>(() =>
    localStorage.getItem(activeStorageKey),
  )
  const [viewedBatchId, setViewedBatchId] = useState<string | null>(() =>
    localStorage.getItem(viewedStorageKey),
  )
  const [resumeTransition, setResumeTransition] =
    useState<ResumeTransition | null>(null)

  /* eslint-disable @eslint-react/set-state-in-effect -- synchronize external persisted batch selection */
  useEffect(() => {
    setBatchId(localStorage.getItem(activeStorageKey))
    setViewedBatchId(localStorage.getItem(viewedStorageKey))
  }, [activeStorageKey, viewedStorageKey])
  const listQuery = useQuery({
    queryKey: trafficTracerBatchListKey(workspaceRoot),
    queryFn: listTrafficTracerBatches,
    enabled,
    refetchInterval: false,
    retry: 1,
  })

  const recoveredBatch = batchId
    ? undefined
    : listQuery.data?.batches.find((batch) => batch.state === 'running')
  const activeBatchId = batchId ?? recoveredBatch?.batch_id ?? null

  useEffect(() => {
    if (recoveredBatch) {
      localStorage.setItem(activeStorageKey, recoveredBatch.batch_id)
      localStorage.removeItem(viewedStorageKey)
      setViewedBatchId(null)
      setBatchId(recoveredBatch.batch_id)
    }
  }, [activeStorageKey, recoveredBatch, viewedStorageKey])

  /* eslint-enable @eslint-react/set-state-in-effect */

  const selectedBatchId = viewedBatchId ?? activeBatchId
  const listedBatch = selectedBatchId
    ? listQuery.data?.batches.find(
        (batch) => batch.batch_id === selectedBatchId,
      )
    : undefined

  const statusQuery = useQuery({
    queryKey: selectedBatchId
      ? trafficTracerBatchKey(selectedBatchId)
      : ['trafficTracer', 'batch', 'none'],
    queryFn: () => getTrafficTracerBatch(selectedBatchId!),
    enabled: enabled && Boolean(selectedBatchId),
    refetchInterval: ({ state }) => {
      const transitionPending =
        resumeTransition?.batchId === selectedBatchId &&
        (!state.data ||
          (state.data.batch.state !== 'running' &&
            state.data.batch.resume.attempt <=
              resumeTransition.baselineAttempt))
      if (transitionPending) return 1000
      return state.data && TERMINAL.has(state.data.batch.state) ? false : 1000
    },
  })

  const resumeTransitionActive =
    resumeTransition?.batchId === selectedBatchId &&
    (!statusQuery.data ||
      (statusQuery.data.batch.state !== 'running' &&
        statusQuery.data.batch.resume.attempt <=
          resumeTransition.baselineAttempt))

  useEffect(() => {
    const state = statusQuery.data?.batch.state
    if (
      selectedBatchId === activeBatchId &&
      state &&
      TERMINAL.has(state) &&
      !resumeTransitionActive
    ) {
      localStorage.removeItem(activeStorageKey)
    }
  }, [
    activeBatchId,
    activeStorageKey,
    resumeTransitionActive,
    selectedBatchId,
    statusQuery.data?.batch.state,
  ])

  const liveStatus = statusQuery.data
  const batchStatus =
    listedBatch &&
    (!liveStatus ||
      Date.parse(listedBatch.updated_at) >=
        Date.parse(liveStatus.batch.updated_at))
      ? { batch: listedBatch, job: liveStatus?.job ?? null }
      : liveStatus

  const remember = (id: string) => {
    localStorage.setItem(activeStorageKey, id)
    localStorage.removeItem(viewedStorageKey)
    setViewedBatchId(null)
    setBatchId(id)
  }
  const selectBatch = (id: string | null) => {
    if (id === null || id === activeBatchId) {
      localStorage.removeItem(viewedStorageKey)
      setViewedBatchId(null)
      return
    }
    localStorage.setItem(viewedStorageKey, id)
    setViewedBatchId(id)
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
  const interruptMutation = useMutation({
    mutationFn: (reason?: string) => {
      if (!activeBatchId) throw new Error('No active TrafficTracer batch')
      return interruptTrafficTracerBatch(activeBatchId, reason)
    },
    onSuccess: (status) => {
      queryClient.setQueryData(
        trafficTracerBatchKey(status.batch.batch_id),
        status,
      )
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
      if (!selectedBatchId)
        throw new Error('No TrafficTracer batch is selected')
      return resumeTrafficTracerBatch(selectedBatchId)
    },
    onMutate: () => {
      if (!selectedBatchId) return
      remember(selectedBatchId)
      setResumeTransition({
        batchId: selectedBatchId,
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
    batchId: selectedBatchId,
    activeBatchId,
    viewedBatchId,
    batchStatus,
    batches: listQuery.data?.batches ?? [],
    corruptBatches: listQuery.data?.corrupt ?? [],
    listQuery,
    statusQuery,
    startBatch: startMutation.mutateAsync,
    startMutation,
    interruptBatch: interruptMutation.mutateAsync,
    interruptMutation,
    cancelBatch: cancelMutation.mutateAsync,
    cancelMutation,
    resumeBatch: resumeMutation.mutateAsync,
    resumeMutation,
    resuming: resumeMutation.isPending || resumeTransitionActive,
    selectBatch,
  }
}
