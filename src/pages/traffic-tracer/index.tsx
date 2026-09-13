import {
  Alert,
  AlertTitle,
  Box,
  Button,
  Chip,
  MenuItem,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { invoke } from '@tauri-apps/api/core'
import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import { TrafficTracerBatchProgress } from '@/components/traffic-tracer/batch-progress'
import { TrafficTracerCaptureForm } from '@/components/traffic-tracer/capture-form'
import { TrafficTracerFlowQueryForm } from '@/components/traffic-tracer/flow-query-form'
import { TrafficTracerJobProgress } from '@/components/traffic-tracer/job-progress'
import PipelineQualityOverview from '@/components/traffic-tracer/pipeline-quality-overview'
import { TrafficTracerPipelineQueue } from '@/components/traffic-tracer/pipeline-queue'
import {
  PIPELINE_MODE_STORAGE_KEY,
  restoredPipelineCandidates,
  restoredPipelineRepetitions,
} from '@/components/traffic-tracer/pipeline-queue-storage'
import { TrafficTracerSessionsView } from '@/components/traffic-tracer/sessions-view'
import { useCaptureJob } from '@/hooks/use-capture-job'
import { useTrafficTracerBatches } from '@/hooks/use-traffic-tracer-batches'
import { useTrafficTracerWorker } from '@/hooks/use-traffic-tracer-worker'
import {
  cancelTrafficTracerPipeline,
  getTrafficTracerBatch,
  getTrafficTracerPipeline,
  interruptTrafficTracerPipeline,
  listTrafficTracerPipelines,
  reconcileTrafficTracerPipeline,
  retryTrafficTracerPipelineRestore,
  resumeTrafficTracerPipeline,
  startTrafficTracerPipeline,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type {
  BatchStartRequest,
  BatchStatusResult,
  CaptureStartRequest,
  EnvironmentRequest,
  PipelineCandidate,
  PipelineListEntry,
  PipelineManifest,
} from '@/types/traffic-tracer'
import { announceDesktopRecovery } from '@/utils/traffic-tracer-desktop-recovery'
import { RECOVERY_CONFIRMATION_RETRY_EVENT } from '@/utils/traffic-tracer-recovery-snapshots'
import { startVisibleSnapshotPoll } from '@/utils/traffic-tracer-visible-poll'

const START_FAILURE_STORAGE_KEY = 'traffictracer.lastStartFailure'
const ENVIRONMENT_REQUEST_STORAGE_KEY = 'traffictracer.environmentRequest.v1'
const ACTIVE_PIPELINE_STORAGE_KEY = 'traffictracer.activePipeline.v1'

type StartFailure = { at: string; stage: string; message: string }

function failureMessage(error: unknown) {
  if (error instanceof Error) return error.message
  if (typeof error === 'string') return error
  try {
    return JSON.stringify(error)
  } catch {
    return String(error)
  }
}

function restoredEnvironmentRequest(): EnvironmentRequest | null {
  const stored = localStorage.getItem(ENVIRONMENT_REQUEST_STORAGE_KEY)
  if (!stored) return null
  try {
    const request = JSON.parse(stored) as Partial<EnvironmentRequest>
    if (
      typeof request.tun_interface !== 'string' ||
      typeof request.physical_interface !== 'string' ||
      typeof request.chrome_binary !== 'string' ||
      typeof request.output_root !== 'string'
    ) {
      throw new Error('invalid persisted environment request')
    }
    return request as EnvironmentRequest
  } catch {
    localStorage.removeItem(ENVIRONMENT_REQUEST_STORAGE_KEY)
    return null
  }
}
const TrafficTracerPage = () => {
  const { t } = useTranslation()
  const [diagnosticRequest, setDiagnosticRequest] =
    useState<EnvironmentRequest | null>(restoredEnvironmentRequest)
  const [startFailure, setStartFailure] = useState<StartFailure | null>(() => {
    const stored = localStorage.getItem(START_FAILURE_STORAGE_KEY)
    if (!stored) return null
    try {
      return JSON.parse(stored) as StartFailure
    } catch {
      localStorage.removeItem(START_FAILURE_STORAGE_KEY)
      return null
    }
  })
  const recordStartFailure = useCallback((error: unknown, stage: string) => {
    const failure = {
      at: new Date().toISOString(),
      stage,
      message: failureMessage(error),
    }
    localStorage.setItem(START_FAILURE_STORAGE_KEY, JSON.stringify(failure))
    // eslint-disable-next-line @eslint-react/set-state-in-effect -- synchronize terminal Worker failures with persisted UI state
    setStartFailure(failure)
  }, [])
  const clearStartFailure = useCallback(() => {
    localStorage.removeItem(START_FAILURE_STORAGE_KEY)
    setStartFailure(null)
  }, [])
  const {
    job,
    jobStartedAt,
    progressEvents,
    startCapture,
    startMutation,
    cancelJob,
    cancelMutation,
    clearJob,
  } = useCaptureJob()
  const batches = useTrafficTracerBatches(
    diagnosticRequest?.output_root ?? '',
    diagnosticRequest !== null,
  )
  const [pipelineEnabled, setPipelineEnabled] = useState(
    () => localStorage.getItem(PIPELINE_MODE_STORAGE_KEY) === 'true',
  )
  const [pipelineCandidates, setPipelineCandidates] = useState<
    PipelineCandidate[]
  >(restoredPipelineCandidates)
  const [pipelineRepetitions, setPipelineRepetitions] = useState(
    restoredPipelineRepetitions,
  )
  const [pipelineTargetCount, setPipelineTargetCount] = useState(0)
  const [pipeline, setPipeline] = useState<PipelineManifest | null>(null)
  const [pipelineNow, setPipelineNow] = useState(() => Date.now())
  const [pipelineBatchStatus, setPipelineBatchStatus] =
    useState<BatchStatusResult | null>(null)
  const [pipelineLocator, setPipelineLocator] = useState<{
    pipeline_id: string
    output_root: string
  } | null>(() => {
    try {
      return JSON.parse(
        localStorage.getItem(ACTIVE_PIPELINE_STORAGE_KEY) ?? 'null',
      )
    } catch {
      return null
    }
  })
  const [pipelineActionPending, setPipelineActionPending] = useState(false)
  const pipelineRequestFenceRef = useRef({ revision: 0, pending: false })
  const updatePipelineActionPending = useCallback((pending: boolean) => {
    // Invalidate reads issued before either edge of a lifecycle operation.
    // The ref changes synchronously, before React commits the pending state.
    pipelineRequestFenceRef.current.revision += 1
    pipelineRequestFenceRef.current.pending = pending
    setPipelineActionPending(pending)
  }, [])
  const [pipelineHistory, setPipelineHistory] = useState<PipelineListEntry[]>(
    [],
  )
  const pipelineTerminal = new Set([
    'completed',
    'completed_with_degraded',
    'completed_with_errors',
    'failed',
    'cancelled',
    'interrupted',
    'restore_failed',
  ])
  const pipelineActive =
    pipeline !== null && !pipelineTerminal.has(pipeline.state)
  const displayedPipelineRun =
    pipeline?.current_run_index !== null &&
    pipeline?.current_run_index !== undefined
      ? pipeline.runs[pipeline.current_run_index]
      : [...(pipeline?.runs ?? [])]
          .reverse()
          .find((run) => run.state !== 'pending')
  const pipelineErrorRun = [...(pipeline?.runs ?? [])]
    .reverse()
    .find((run) => run.error !== null)
  const pipelineError = displayedPipelineRun?.error ?? pipelineErrorRun?.error
  const displayedBatch =
    pipelineBatchStatus &&
    displayedPipelineRun?.batch_id === pipelineBatchStatus.batch.batch_id
      ? pipelineBatchStatus.batch
      : null
  const displayedBatchIndex =
    displayedBatch?.current_index ?? displayedBatch?.resume.next_index ?? null
  const displayedTarget =
    (displayedBatchIndex !== null && displayedBatchIndex !== undefined
      ? displayedBatch?.targets[displayedBatchIndex]
      : null) ??
    pipeline?.targets.find(
      (target) => target.index === displayedPipelineRun?.target_index,
    ) ??
    null
  const capturedCellCount =
    pipeline?.runs.filter(
      (run) =>
        run.session_ids.length > 0 ||
        run.prior_session_ids.length > 0 ||
        ['captured', 'analyzing', 'completed', 'degraded'].includes(run.state),
    ).length ?? 0
  const analyzedCellCount =
    pipeline?.runs.filter(
      (run) =>
        ['completed', 'degraded'].includes(run.state) ||
        (run.state === 'failed' && Boolean(run.analysis_job_id)),
    ).length ?? 0
  const pipelineElapsedSeconds = displayedPipelineRun?.started_at
    ? Math.max(
        0,
        Math.floor(
          (pipelineNow - Date.parse(displayedPipelineRun.started_at)) / 1000,
        ),
      )
    : null
  const {
    environment,
    environmentQuery,
    captureLock,
    workerActivity,
    checkEnvironment,
  } = useTrafficTracerWorker(
    diagnosticRequest,
    true,
    pipelineActionPending || pipelineActive,
  )

  const handleDiagnose = (request: EnvironmentRequest) => {
    localStorage.setItem(
      ENVIRONMENT_REQUEST_STORAGE_KEY,
      JSON.stringify(request),
    )
    setDiagnosticRequest(request)
    void checkEnvironment(request).catch((error: unknown) =>
      recordStartFailure(error, 'environment_check'),
    )
  }

  useEffect(() => {
    if (!pipelineLocator) return
    let disposed = false
    let inFlight = false
    const refresh = async () => {
      if (disposed || inFlight || pipelineRequestFenceRef.current.pending)
        return false
      inFlight = true
      const revision = pipelineRequestFenceRef.current.revision
      try {
        const current = await getTrafficTracerPipeline(
          pipelineLocator.output_root,
        )
        const run =
          current.current_run_index !== null
            ? current.runs[current.current_run_index]
            : [...current.runs]
                .reverse()
                .find((item) => item.state !== 'pending')
        let batchStatus: BatchStatusResult | null = null
        let batchReadSucceeded = true
        if (run?.batch_id) {
          try {
            batchStatus = await getTrafficTracerBatch(run.batch_id)
          } catch {
            batchStatus = null
            batchReadSucceeded = false
          }
        }
        if (
          !disposed &&
          revision === pipelineRequestFenceRef.current.revision
        ) {
          setPipeline(current)
          setPipelineBatchStatus(batchStatus)
          setPipelineNow(Date.now())
          return batchReadSucceeded
        }
      } catch (error) {
        if (
          !disposed &&
          revision === pipelineRequestFenceRef.current.revision
        ) {
          recordStartFailure(error, 'pipeline.status')
        }
      } finally {
        inFlight = false
      }
      return false
    }
    const stopPolling = startVisibleSnapshotPoll(refresh, 1000)
    return () => {
      disposed = true
      stopPolling()
    }
  }, [pipelineLocator, recordStartFailure])
  useEffect(() => {
    const outputRoot = diagnosticRequest?.output_root
    if (!outputRoot) return
    let disposed = false
    let inFlight = false
    const refresh = async () => {
      if (disposed || inFlight) return false
      inFlight = true
      try {
        const entries = await listTrafficTracerPipelines(outputRoot)
        if (!disposed) {
          setPipelineHistory(entries)
          return true
        }
      } catch {
        if (!disposed) setPipelineHistory([])
      } finally {
        inFlight = false
      }
      return false
    }
    const stopPolling = startVisibleSnapshotPoll(refresh, 5000)
    return () => {
      disposed = true
      stopPolling()
    }
  }, [diagnosticRequest?.output_root, pipelineLocator])

  const terminalJobStates = new Set([
    'completed',
    'failed',
    'cancelled',
    'interrupted',
  ])
  const activeJobId =
    job && !terminalJobStates.has(job.state) ? job.job_id : null
  const activeBatchId =
    pipeline && !pipelineTerminal.has(pipeline.state)
      ? (pipeline.runs[pipeline.current_run_index ?? -1]?.batch_id ?? null)
      : batches.batchStatus &&
          !terminalJobStates.has(batches.batchStatus.batch.state)
        ? batches.batchStatus.batch.batch_id
        : null

  useEffect(() => {
    let mounted = true
    let pending = false
    const observeDesktop = (value: unknown) => {
      window.dispatchEvent(new Event(RECOVERY_CONFIRMATION_RETRY_EVENT))
      if (!value || typeof value !== 'object') return
      const snapshot = value as Record<string, unknown>
      if (snapshot.session_lock === 'unlocked')
        announceDesktopRecovery(snapshot.unlock_generation)
    }
    const heartbeat = () => {
      if (pending) return
      const active = mounted && document.visibilityState === 'visible'
      pending = true
      void invoke('tt_ui_heartbeat', { active })
        .then((snapshot) => {
          if (mounted) observeDesktop(snapshot)
        })
        .catch(() => undefined)
        .finally(() => {
          pending = false
        })
    }
    heartbeat()
    const interval = window.setInterval(heartbeat, 2000)
    document.addEventListener('visibilitychange', heartbeat)
    return () => {
      mounted = false
      window.clearInterval(interval)
      document.removeEventListener('visibilitychange', heartbeat)
      void invoke('tt_ui_heartbeat', { active: false }).catch(() => undefined)
    }
  }, [])

  const handleStartCapture = async (request: CaptureStartRequest) => {
    try {
      await startCapture(request)
      batches.clearBatch()
      clearStartFailure()
      showNotice.success('settings.trafficTracer.notifications.captureStarted')
    } catch (error) {
      recordStartFailure(error, 'capture.start')
      showNotice.error(error)
    }
  }

  const handleCancel = async (reason?: string) => {
    try {
      await cancelJob({ reason })
    } catch (error) {
      showNotice.error(error)
    }
  }

  const handleStartBatch = async (request: BatchStartRequest) => {
    try {
      await batches.startBatch(request)
      clearJob()
      clearStartFailure()
      showNotice.success('settings.trafficTracer.notifications.captureStarted')
    } catch (error) {
      recordStartFailure(error, 'batch.start')
      showNotice.error(error)
    }
  }

  const handleStartPipeline = async (batch: BatchStartRequest) => {
    if (pipelineRequestFenceRef.current.pending) return
    updatePipelineActionPending(true)
    try {
      const started = await startTrafficTracerPipeline({
        batch,
        candidates: pipelineCandidates,
        repetitions_per_candidate: pipelineRepetitions,
        continue_on_run_failure: true,
        candidate_order_policy: 'balanced_seeded',
      })
      const locator = {
        pipeline_id: started.pipeline_id,
        output_root: started.output_root,
      }
      localStorage.setItem(ACTIVE_PIPELINE_STORAGE_KEY, JSON.stringify(locator))
      setPipelineLocator(locator)
      setPipeline(started)
      clearJob()
      batches.clearBatch()
      clearStartFailure()
      showNotice.success('TrafficTracer pipeline started.')
    } catch (error) {
      recordStartFailure(error, 'pipeline.start')
      showNotice.error(error)
    } finally {
      updatePipelineActionPending(false)
    }
  }

  const handleResumePipeline = async () => {
    if (!pipelineLocator || pipelineRequestFenceRef.current.pending) return
    updatePipelineActionPending(true)
    try {
      const resumed = await resumeTrafficTracerPipeline(
        pipelineLocator.output_root,
      )
      setPipeline(resumed)
      clearStartFailure()
      showNotice.success('TrafficTracer pipeline resumed.')
    } catch (error) {
      recordStartFailure(error, 'pipeline.resume')
      showNotice.error(error)
    } finally {
      updatePipelineActionPending(false)
    }
  }

  const handleRetryPipelineRestore = async () => {
    if (!pipelineLocator || pipelineRequestFenceRef.current.pending) return
    updatePipelineActionPending(true)
    try {
      const restored = await retryTrafficTracerPipelineRestore(
        pipelineLocator.output_root,
      )
      setPipeline(restored)
      if (restored.state === 'restore_failed') {
        const error = restored.restore.error ?? {
          code: 'RESTORE_FAILED',
          message: 'Pipeline restoration readback did not pass.',
        }
        recordStartFailure(error, 'pipeline.restore')
        showNotice.error(error)
        return
      }
      clearStartFailure()
      showNotice.success('TrafficTracer pipeline restoration completed.')
    } catch (error) {
      recordStartFailure(error, 'pipeline.restore')
      showNotice.error(error)
    } finally {
      updatePipelineActionPending(false)
    }
  }

  const handleReconcilePipeline = async () => {
    if (!pipelineLocator || pipelineRequestFenceRef.current.pending) return
    updatePipelineActionPending(true)
    try {
      const reconciled = await reconcileTrafficTracerPipeline(
        pipelineLocator.output_root,
      )
      setPipeline(reconciled)
      clearStartFailure()
      showNotice.success('Existing pipeline analyses reconciled.')
    } catch (error) {
      recordStartFailure(error, 'pipeline.reconcile')
      showNotice.error(error)
    } finally {
      updatePipelineActionPending(false)
    }
  }

  const handlePipelineStop = async (cancel: boolean) => {
    if (!pipeline || pipelineRequestFenceRef.current.pending) return
    updatePipelineActionPending(true)
    try {
      const updated = cancel
        ? await cancelTrafficTracerPipeline(pipeline.pipeline_id)
        : await interruptTrafficTracerPipeline(pipeline.pipeline_id)
      setPipeline(updated)
      clearStartFailure()
    } catch (error) {
      recordStartFailure(
        error,
        cancel ? 'pipeline.cancel' : 'pipeline.interrupt',
      )
      showNotice.error(error)
    } finally {
      updatePipelineActionPending(false)
    }
  }
  useEffect(() => {
    if (job?.state === 'failed') {
      recordStartFailure(
        job.error ?? job.message,
        job.stage || 'capture.runtime',
      )
    }
  }, [job, recordStartFailure])

  useEffect(() => {
    const status = batches.batchStatus
    if (
      status?.batch.state !== 'failed' ||
      batches.viewedBatchId ||
      status.batch.batch_id !== batches.activeBatchId
    )
      return
    const child = [...status.batch.children]
      .reverse()
      .find((item) => item.error)
    recordStartFailure(
      child?.error ??
        status.job?.error ??
        status.job?.message ??
        'Capture Group failed',
      status.job?.stage || 'batch.runtime',
    )
  }, [
    batches.activeBatchId,
    batches.batchStatus,
    batches.viewedBatchId,
    recordStartFailure,
  ])

  useEffect(() => {
    if (!pipeline || !['failed', 'restore_failed'].includes(pipeline.state))
      return
    const failedRun = [...pipeline.runs].reverse().find((run) => run.error)
    const error = failedRun?.error ??
      pipeline.restore.error ?? {
        code: 'PIPELINE_FAILED',
        message: 'The capture pipeline terminated before completion.',
      }
    const message = failureMessage(error)
    if (
      startFailure?.stage === 'pipeline.runtime' &&
      startFailure.message === message
    )
      return
    recordStartFailure(error, 'pipeline.runtime')
  }, [pipeline, recordStartFailure, startFailure])

  return (
    <BasePage title={t('layout.components.navigation.tabs.trafficTracer')}>
      <Box data-testid="traffic-tracer-workspace" sx={{ pb: 2 }}>
        {workerActivity && (
          <Alert severity="info" sx={{ mb: 2 }}>
            <AlertTitle>Worker preparation</AlertTitle>
            {workerActivity.message} Recovery{' '}
            {(workerActivity.timing.duration_ms / 1000).toFixed(2)}s
            {workerActivity.timing.catalog?.operation && (
              <Box component="span" sx={{ display: 'block', opacity: 0.8 }}>
                Session catalog: {workerActivity.timing.catalog.operation} ·{' '}
                {(
                  Number(workerActivity.timing.catalog.duration_ms ?? 0) / 1000
                ).toFixed(2)}
                s
              </Box>
            )}
          </Alert>
        )}
        {startFailure && (
          <Alert severity="error" onClose={clearStartFailure} sx={{ mb: 2 }}>
            <AlertTitle>
              {['pipeline.interrupt', 'pipeline.cancel'].includes(
                startFailure.stage,
              )
                ? 'Pipeline control request failed'
                : 'Capture did not start or terminated early'}
            </AlertTitle>
            {startFailure.message}
            <Box
              component="span"
              sx={{ display: 'block', mt: 0.5, opacity: 0.8 }}
            >
              {startFailure.stage} ·{' '}
              {new Date(startFailure.at).toLocaleString()}
            </Box>
          </Alert>
        )}
        {job && (
          <Box sx={{ mb: 2 }}>
            <TrafficTracerJobProgress
              job={job}
              startedAt={jobStartedAt}
              events={progressEvents}
              cancelling={cancelMutation.isPending}
              onCancel={handleCancel}
            />
          </Box>
        )}
        {batches.batches.length > 0 && (
          <Box sx={{ mb: 2, maxWidth: 520 }}>
            <TextField
              select
              fullWidth
              size="small"
              label="Capture Group history"
              disabled={
                batches.batchStatus?.batch.state === 'running' &&
                !batches.viewedBatchId
              }
              value={batches.viewedBatchId ?? ''}
              onChange={(event) =>
                batches.selectBatch(event.target.value || null)
              }
            >
              <MenuItem value="">Current capture</MenuItem>
              {batches.batches.map((batch) => (
                <MenuItem key={batch.batch_id} value={batch.batch_id}>
                  {new Date(batch.created_at).toLocaleString()} · {batch.state}{' '}
                  ·{' '}
                  {
                    batch.children.filter(
                      (child) => child.state === 'completed',
                    ).length
                  }
                  /{batch.targets.length}
                </MenuItem>
              ))}
            </TextField>
          </Box>
        )}
        {batches.batchStatus && (
          <Box sx={{ mb: 2 }}>
            <TrafficTracerBatchProgress
              status={batches.batchStatus}
              workspaceRoot={diagnosticRequest?.output_root ?? ''}
              interrupting={batches.interruptMutation.isPending}
              resuming={batches.resuming}
              canInterrupt={
                batches.batchStatus.batch.batch_id === batches.activeBatchId
              }
              viewingHistory={Boolean(batches.viewedBatchId)}
              onInterrupt={() =>
                void batches.interruptBatch(
                  'Interrupted from the TrafficTracer workspace.',
                )
              }
              onResume={() => void batches.resumeBatch()}
            />
          </Box>
        )}
        {pipelineHistory.length > 0 && (
          <Box sx={{ mb: 2, maxWidth: 720 }}>
            <TextField
              select
              fullWidth
              size="small"
              label="Profile / node pipeline history"
              disabled={pipelineActionPending}
              value={pipelineLocator?.output_root ?? ''}
              onChange={(event) => {
                const selected = pipelineHistory.find(
                  (item) => item.output_root === event.target.value,
                )
                if (!selected) return
                const locator = {
                  pipeline_id: selected.pipeline_id,
                  output_root: selected.output_root,
                }
                localStorage.setItem(
                  ACTIVE_PIPELINE_STORAGE_KEY,
                  JSON.stringify(locator),
                )
                setPipelineLocator(locator)
              }}
            >
              {pipelineHistory.map((item) => (
                <MenuItem key={item.pipeline_id} value={item.output_root}>
                  {new Date(item.updated_at).toLocaleString()} · {item.state} ·{' '}
                  {item.completed_runs}/{item.total_runs} ·{' '}
                  {item.candidate_count} nodes ×{' '}
                  {item.repetitions_per_candidate}
                </MenuItem>
              ))}
            </TextField>
          </Box>
        )}
        {pipeline && (
          <Alert
            severity={
              pipeline.state === 'failed' || pipeline.state === 'restore_failed'
                ? 'error'
                : pipeline.state === 'completed_with_errors' ||
                    pipeline.state === 'completed_with_degraded'
                  ? 'warning'
                  : 'info'
            }
            sx={{ mb: 2 }}
          >
            <AlertTitle>Profile / node pipeline</AlertTitle>
            {pipelineActive &&
              displayedPipelineRun?.stage === 'activating_profile' &&
              pipelineNow - Date.parse(pipeline.updated_at) > 90000 && (
                <Alert severity="error" sx={{ mb: 1 }}>
                  PROFILE_ACTIVATION_STALLED: no durable checkpoint for over 90
                  seconds. The supervisor requests interruption without starting
                  another capture or switching another node. Preserve this
                  capture group for recovery.
                </Alert>
              )}
            <Box>
              {pipeline.state} · {pipeline.stage.replaceAll('_', ' ')} ·{' '}
              {pipelineActive ? 'Current task' : 'Last task'}{' '}
              {pipeline.current_run_index !== null
                ? pipeline.current_run_index + 1
                : (displayedPipelineRun?.ordinal ?? 0)}
              /{pipeline.runs.length}
            </Box>
            <PipelineQualityOverview pipeline={pipeline} />
            <Box sx={{ opacity: 0.8 }}>
              Captured {capturedCellCount}/{pipeline.runs.length} cells ·
              analyzed {analyzedCellCount}/{pipeline.runs.length}
              {' · '}awaiting analysis{' '}
              {pipeline.runs.filter((run) => run.state === 'captured').length}
              {' · '}pending{' '}
              {pipeline.runs.filter((run) => run.state === 'pending').length}
              {' · '}interrupted{' '}
              {
                pipeline.runs.filter((run) => run.state === 'interrupted')
                  .length
              }
            </Box>
            {pipeline.schedule.mode === 'repetition_target_candidate' && (
              <Box sx={{ opacity: 0.65 }}>
                Candidate order: balanced seeded · seed{' '}
                {pipeline.schedule.random_seed ?? 'none'} · current repetition{' '}
                {displayedPipelineRun?.repetition_index ?? 1}:{' '}
                {pipeline.schedule.repetition_candidate_orders[
                  (displayedPipelineRun?.repetition_index ?? 1) - 1
                ]?.join(' → ') ?? 'unavailable'}
              </Box>
            )}
            <Box sx={{ opacity: 0.65, overflowWrap: 'anywhere' }}>
              {pipelineLocator?.output_root}
            </Box>
            <details
              key={`${pipeline.pipeline_id}:${pipelineActive}`}
              open={pipelineActive}
            >
              <summary>
                {pipelineActive
                  ? 'Current sample details'
                  : 'Last sample details (not whole pipeline quality)'}
              </summary>
              {displayedPipelineRun && (
                <Stack spacing={0.5} sx={{ mt: 0.75 }}>
                  <Box sx={{ opacity: 0.8 }}>
                    Node #{displayedPipelineRun.candidate_ordinal} ·{' '}
                    {displayedPipelineRun.observed_protocol ||
                      displayedPipelineRun.expected_protocol ||
                      'protocol not observed'}{' '}
                    · repetition {displayedPipelineRun.repetition_index}/
                    {displayedPipelineRun.repetition_total}
                  </Box>
                  <Box sx={{ opacity: 0.8 }}>
                    {displayedPipelineRun.profile_uid} ·{' '}
                    {displayedPipelineRun.selection_group} ·{' '}
                    {displayedPipelineRun.requested_node}
                  </Box>
                  <Box sx={{ opacity: 0.8 }}>
                    Run {displayedPipelineRun.state} ·{' '}
                    {displayedPipelineRun.stage.replaceAll('_', ' ')}
                    {pipelineElapsedSeconds !== null &&
                      ` · elapsed ${pipelineElapsedSeconds}s`}
                  </Box>
                  {displayedPipelineRun.selected_attempt !== null &&
                    displayedPipelineRun.selected_attempt !== undefined && (
                      <Box sx={{ opacity: 0.8 }}>
                        Selected attempt #
                        {displayedPipelineRun.selected_attempt}/
                        {displayedPipelineRun.attempts?.length ?? 1}
                        {displayedPipelineRun.attempts?.find(
                          (attempt) => attempt.selected,
                        )?.selection_reason &&
                          ` · ${displayedPipelineRun.attempts
                            .find((attempt) => attempt.selected)!
                            .selection_reason!.replaceAll('_', ' ')}`}
                      </Box>
                    )}
                  <Box sx={{ opacity: 0.65 }}>
                    Last durable checkpoint:{' '}
                    {new Date(pipeline.updated_at).toLocaleString()}
                  </Box>
                  {displayedPipelineRun.profile_bound_at && (
                    <Box sx={{ opacity: 0.8, overflowWrap: 'anywhere' }}>
                      Configuration snapshot:{' '}
                      {displayedPipelineRun.profile_fingerprint_kind.replaceAll(
                        '_',
                        ' ',
                      )}{' '}
                      · {displayedPipelineRun.profile_fingerprint.slice(0, 12)}{' '}
                      ·{' '}
                      {new Date(
                        displayedPipelineRun.profile_bound_at,
                      ).toLocaleString()}
                      {displayedPipelineRun.profile_snapshot_changed &&
                        ' · rebound from queued snapshot'}
                    </Box>
                  )}
                  {displayedPipelineRun.evidence?.profile_activation && (
                    <Box sx={{ opacity: 0.8, overflowWrap: 'anywhere' }}>
                      Profile activation:{' '}
                      {displayedPipelineRun.evidence.profile_activation.last_completed_step.replaceAll(
                        '_',
                        ' ',
                      )}
                      {' · '}
                      {displayedPipelineRun.evidence.profile_activation
                        .source_profile_uid ?? 'none'}
                      {' → '}
                      {
                        displayedPipelineRun.evidence.profile_activation
                          .target_profile_uid
                      }
                      {displayedPipelineRun.evidence.profile_activation
                        .resumed_from_committed_state &&
                        ' · resumed from committed Profile'}
                    </Box>
                  )}
                </Stack>
              )}
              {displayedBatch && (
                <Box sx={{ mt: 1 }}>
                  <Typography variant="body2">
                    Target{' '}
                    {Math.min(
                      (displayedBatchIndex ??
                        displayedBatch.targets.length - 1) + 1,
                      displayedBatch.targets.length,
                    )}
                    /{displayedBatch.targets.length} ·{' '}
                    {displayedBatch.stage.replaceAll('_', ' ')} · attempt{' '}
                    {displayedBatch.resume.attempt + 1}
                  </Typography>
                  {displayedTarget && (
                    <Typography
                      variant="body2"
                      title={displayedTarget.url}
                      sx={{ overflowWrap: 'anywhere', opacity: 0.8 }}
                    >
                      {displayedTarget.domain} — {displayedTarget.url}
                    </Typography>
                  )}
                </Box>
              )}
              {displayedPipelineRun?.quality && (
                <Stack
                  direction="row"
                  spacing={0.75}
                  useFlexGap
                  sx={{ mt: 1, flexWrap: 'wrap' }}
                >
                  {(
                    [
                      ...(displayedPipelineRun.quality.local_runtime
                        ? ([
                            [
                              'Local runtime',
                              displayedPipelineRun.quality.local_runtime,
                            ],
                          ] as const)
                        : []),
                      [
                        'Capture',
                        displayedPipelineRun.quality.capture_integrity,
                      ],
                      ['Correlation', displayedPipelineRun.quality.correlation],
                      ['Application', displayedPipelineRun.quality.application],
                    ] as const
                  ).map(([label, quality]) => (
                    <Chip
                      key={label}
                      size="small"
                      label={`Sample ${label}: ${quality.state.replaceAll('_', ' ')}`}
                      color={
                        quality.state === 'failed'
                          ? 'error'
                          : quality.state === 'degraded' ||
                              quality.state === 'indeterminate'
                            ? 'warning'
                            : quality.state === 'passed'
                              ? 'success'
                              : 'default'
                      }
                    />
                  ))}
                </Stack>
              )}
              {displayedPipelineRun?.evidence?.drain && (
                <Box sx={{ mt: 1, opacity: 0.8 }}>
                  Connection drain:{' '}
                  {displayedPipelineRun.evidence.drain.state.replaceAll(
                    '_',
                    ' ',
                  )}
                  {' · '}
                  {displayedPipelineRun.evidence.drain.initial_connections ??
                    '?'}
                  {' → '}
                  {displayedPipelineRun.evidence.drain.final_connections ?? '?'}
                  {' · '}
                  {displayedPipelineRun.evidence.drain.quiet_millis}ms quiet
                </Box>
              )}
              {displayedPipelineRun?.evidence?.verification && (
                <Stack
                  direction="row"
                  spacing={0.75}
                  useFlexGap
                  sx={{ mt: 1, flexWrap: 'wrap' }}
                >
                  {(
                    [
                      [
                        'Node evidence',
                        displayedPipelineRun.evidence.verification.node_state,
                      ],
                      [
                        'Protocol evidence',
                        displayedPipelineRun.evidence.verification
                          .protocol_state,
                      ],
                    ] as const
                  ).map(([label, state]) => (
                    <Chip
                      key={label}
                      size="small"
                      label={`${label}: ${state.replaceAll('_', ' ')}`}
                      color={
                        state === 'node_drift' || state === 'protocol_mismatch'
                          ? 'error'
                          : state === 'observation_unavailable'
                            ? 'warning'
                            : 'success'
                      }
                    />
                  ))}
                </Stack>
              )}
              {displayedPipelineRun?.evidence?.verification?.details.map(
                (detail) => (
                  <Box key={detail} sx={{ mt: 0.5, opacity: 0.75 }}>
                    {detail}
                  </Box>
                ),
              )}
              {displayedPipelineRun?.quality?.application_issues.map(
                (issue) => (
                  <Box
                    key={`${issue.session_id}:${issue.target_url}`}
                    sx={{ mt: 1, overflowWrap: 'anywhere' }}
                  >
                    Application {issue.state}:{' '}
                    {issue.reason ?? 'unknown outcome'}
                    {issue.origin &&
                      ` · source ${issue.origin.replaceAll('_', ' ')}`}
                    {issue.retryable != null &&
                      ` · ${issue.retryable ? 'retryable' : 'not retryable'}`}
                    {issue.primary_content_millis !== null &&
                      issue.desired_primary_seconds !== null &&
                      ` · ${(issue.primary_content_millis / 1000).toFixed(3)}/${issue.desired_primary_seconds}s primary content`}
                    {issue.final_status != null && (
                      <Box sx={{ opacity: 0.7 }}>
                        Final HTTP status: {issue.final_status}
                      </Box>
                    )}
                    {issue.final_url &&
                      issue.final_url !== issue.target_url && (
                        <Box sx={{ opacity: 0.7 }}>
                          Final URL: {issue.final_url}
                        </Box>
                      )}
                  </Box>
                ),
              )}
              {(displayedPipelineRun?.attempts?.length ?? 0) > 1 && (
                <details>
                  <summary>
                    Attempt history ({displayedPipelineRun!.attempts.length})
                  </summary>
                  <Stack spacing={0.5} sx={{ mt: 0.75 }}>
                    {displayedPipelineRun!.attempts.map((attempt) => (
                      <Box
                        key={attempt.ordinal}
                        sx={{ opacity: attempt.selected ? 1 : 0.72 }}
                      >
                        Attempt #{attempt.ordinal} · {attempt.state}
                        {attempt.selected && ' · selected'}
                        {' · '}application{' '}
                        {attempt.quality?.application.state ?? 'unavailable'}
                        {' · '}capture{' '}
                        {attempt.quality?.capture_integrity.state ??
                          'unavailable'}
                        {' · '}correlation{' '}
                        {attempt.quality?.correlation.state ?? 'unavailable'}
                        {' · '}Sessions {attempt.session_ids.length}
                        {attempt.error &&
                          ` · ${attempt.error.code}: ${attempt.error.message}`}
                      </Box>
                    ))}
                  </Stack>
                </details>
              )}
            </details>
            {pipeline.cleanup && pipeline.cleanup.state !== 'completed' && (
              <Alert
                severity={
                  pipeline.cleanup.state === 'warning' ? 'warning' : 'info'
                }
              >
                {pipeline.cleanup.message ??
                  'Analysis results are saved. Finishing Worker cleanup…'}
                {pipeline.cleanup.state === 'warning' &&
                  pipeline.cleanup.capture_lock_retained &&
                  ' Capture remains locked because Worker cleanup or process exit has not been confirmed.'}
              </Alert>
            )}
            {pipelineError && (
              <Box
                sx={{
                  mt: 1,
                  fontFamily: 'monospace',
                  overflowWrap: 'anywhere',
                }}
              >
                {pipelineErrorRun &&
                  pipelineErrorRun.run_id !== displayedPipelineRun?.run_id && (
                    <Box sx={{ mb: 0.5, opacity: 0.8 }}>
                      Latest pipeline failure: {pipelineErrorRun.profile_uid} ·{' '}
                      {pipelineErrorRun.selection_group} ·{' '}
                      {pipelineErrorRun.requested_node}
                    </Box>
                  )}
                {pipelineError.code}: {pipelineError.message}
              </Box>
            )}
            {pipeline.restore.error && (
              <Box
                sx={{
                  mt: 1,
                  fontFamily: 'monospace',
                  overflowWrap: 'anywhere',
                }}
              >
                Restore {pipeline.restore.error.code}:{' '}
                {pipeline.restore.error.message}
              </Box>
            )}
            {pipeline.restore.checks
              .filter((check) => check.state !== 'passed')
              .map((check) => (
                <Box
                  key={`${check.component}:${check.target}:${check.checked_at}`}
                  sx={{ mt: 0.5, opacity: 0.8, overflowWrap: 'anywhere' }}
                >
                  Restore {check.component} {check.target}:{' '}
                  {check.state.replaceAll('_', ' ')}
                  {check.observed && ` · observed ${check.observed}`}
                </Box>
              ))}
            {pipeline.state === 'interrupted' && (
              <Button
                size="small"
                variant="contained"
                sx={{ mt: 1 }}
                disabled={pipelineActionPending || Boolean(captureLock?.locked)}
                onClick={() => void handleResumePipeline()}
              >
                Resume pipeline
              </Button>
            )}
            {pipeline.state === 'restore_failed' && (
              <Button
                size="small"
                variant="contained"
                sx={{ mt: 1 }}
                disabled={pipelineActionPending || Boolean(captureLock?.locked)}
                onClick={() => void handleRetryPipelineRestore()}
              >
                Retry restoration
              </Button>
            )}
            {pipelineTerminal.has(pipeline.state) && (
              <Box sx={{ mt: 1 }}>
                <Button
                  size="small"
                  variant="outlined"
                  disabled={
                    pipelineActionPending || Boolean(captureLock?.locked)
                  }
                  onClick={() => void handleReconcilePipeline()}
                >
                  Reconcile existing analyses
                </Button>
                <Box sx={{ mt: 0.5, opacity: 0.65 }}>
                  Reads current analysis generations and refreshes attempt
                  selection. It does not recapture traffic or start analysis.
                </Box>
              </Box>
            )}
            {!pipelineTerminal.has(pipeline.state) && (
              <Stack direction="row" spacing={1} sx={{ mt: 1 }}>
                <Button
                  size="small"
                  variant="outlined"
                  disabled={pipelineActionPending}
                  onClick={() => void handlePipelineStop(false)}
                >
                  Interrupt
                </Button>
                <Button
                  size="small"
                  color="error"
                  disabled={pipelineActionPending}
                  onClick={() => void handlePipelineStop(true)}
                >
                  Cancel
                </Button>
              </Stack>
            )}
          </Alert>
        )}
        <TrafficTracerPipelineQueue
          enabled={pipelineEnabled}
          candidates={pipelineCandidates}
          repetitions={pipelineRepetitions}
          targetCount={pipelineTargetCount}
          disabled={Boolean(captureLock?.locked)}
          onEnabledChange={setPipelineEnabled}
          onRepetitionsChange={setPipelineRepetitions}
          onChange={setPipelineCandidates}
        />
        <TrafficTracerCaptureForm
          environment={environment}
          diagnosticRequest={diagnosticRequest}
          diagnosing={environmentQuery.isFetching}
          diagnosticError={environmentQuery.error}
          captureLocked={captureLock?.locked}
          submitting={
            startMutation.isPending ||
            batches.startMutation.isPending ||
            pipelineActionPending
          }
          onDiagnose={handleDiagnose}
          onRetryDiagnostics={() => {
            if (diagnosticRequest) handleDiagnose(diagnosticRequest)
          }}
          onSubmit={handleStartCapture}
          onSubmitBatch={handleStartBatch}
          pipelineEnabled={pipelineEnabled}
          pipelineCandidateCount={pipelineCandidates.length}
          pipelineRepetitions={pipelineRepetitions}
          onSelectedTargetCountChange={setPipelineTargetCount}
          onSubmitPipeline={handleStartPipeline}
        />
        <Box sx={{ mt: 2 }}>
          <TrafficTracerSessionsView
            key={diagnosticRequest?.output_root ?? 'sessions-disabled'}
            enabled={environment !== undefined}
            workspaceRoot={diagnosticRequest?.output_root ?? ''}
            activeJobId={activeJobId}
            activeBatchId={activeBatchId}
          />
        </Box>
        <Box sx={{ mt: 2 }}>
          <TrafficTracerFlowQueryForm
            key={diagnosticRequest?.output_root ?? 'flows-disabled'}
            enabled={environment !== undefined}
            workspaceRoot={diagnosticRequest?.output_root ?? ''}
          />
        </Box>
      </Box>
    </BasePage>
  )
}

export default TrafficTracerPage
