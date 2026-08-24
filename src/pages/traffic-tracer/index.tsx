import { Alert, AlertTitle, Box, MenuItem, TextField } from '@mui/material'
import { invoke } from '@tauri-apps/api/core'
import { useCallback, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import { TrafficTracerBatchProgress } from '@/components/traffic-tracer/batch-progress'
import { TrafficTracerCaptureForm } from '@/components/traffic-tracer/capture-form'
import { TrafficTracerFlowQueryForm } from '@/components/traffic-tracer/flow-query-form'
import { TrafficTracerJobProgress } from '@/components/traffic-tracer/job-progress'
import { TrafficTracerSessionsView } from '@/components/traffic-tracer/sessions-view'
import { useCaptureJob } from '@/hooks/use-capture-job'
import { useTrafficTracerBatches } from '@/hooks/use-traffic-tracer-batches'
import { useTrafficTracerWorker } from '@/hooks/use-traffic-tracer-worker'
import { showNotice } from '@/services/notice-service'
import type {
  CaptureStartRequest,
  BatchStartRequest,
  EnvironmentRequest,
} from '@/types/traffic-tracer'

const START_FAILURE_STORAGE_KEY = 'traffictracer.lastStartFailure'
const ENVIRONMENT_REQUEST_STORAGE_KEY = 'traffictracer.environmentRequest.v1'

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
  const handleDiagnose = useCallback((request: EnvironmentRequest) => {
    localStorage.setItem(
      ENVIRONMENT_REQUEST_STORAGE_KEY,
      JSON.stringify(request),
    )
    setDiagnosticRequest(request)
  }, [])
  const { environment, environmentQuery, captureLock } =
    useTrafficTracerWorker(diagnosticRequest)
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
  const terminalJobStates = new Set([
    'completed',
    'failed',
    'cancelled',
    'interrupted',
  ])
  const activeJobId =
    job && !terminalJobStates.has(job.state) ? job.job_id : null
  const activeBatchId =
    batches.batchStatus &&
    !terminalJobStates.has(batches.batchStatus.batch.state)
      ? batches.batchStatus.batch.batch_id
      : null

  useEffect(() => {
    let mounted = true
    const heartbeat = () => {
      const active = mounted && document.visibilityState === 'visible'
      void invoke('tt_ui_heartbeat', { active }).catch(() => undefined)
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

  return (
    <BasePage title={t('layout.components.navigation.tabs.trafficTracer')}>
      <Box data-testid="traffic-tracer-workspace" sx={{ pb: 2 }}>
        {startFailure && (
          <Alert severity="error" onClose={clearStartFailure} sx={{ mb: 2 }}>
            <AlertTitle>Capture did not start or terminated early</AlertTitle>
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
        <TrafficTracerCaptureForm
          environment={environment}
          diagnosticRequest={diagnosticRequest}
          diagnosing={environmentQuery.isFetching}
          diagnosticError={environmentQuery.error}
          captureLocked={captureLock?.locked}
          submitting={
            startMutation.isPending || batches.startMutation.isPending
          }
          onDiagnose={handleDiagnose}
          onRetryDiagnostics={() => void environmentQuery.refetch()}
          onSubmit={handleStartCapture}
          onSubmitBatch={handleStartBatch}
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
