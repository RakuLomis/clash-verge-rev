import { Box } from '@mui/material'
import { invoke } from '@tauri-apps/api/core'
import { useEffect, useState } from 'react'
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

const TrafficTracerPage = () => {
  const { t } = useTranslation()
  const [diagnosticRequest, setDiagnosticRequest] =
    useState<EnvironmentRequest | null>(null)
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
  } = useCaptureJob()
  const batches = useTrafficTracerBatches(
    diagnosticRequest?.output_root ?? '',
    environment !== undefined,
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
      showNotice.success('settings.trafficTracer.notifications.captureStarted')
    } catch (error) {
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
      showNotice.success('settings.trafficTracer.notifications.captureStarted')
    } catch (error) {
      showNotice.error(error)
    }
  }

  return (
    <BasePage title={t('layout.components.navigation.tabs.trafficTracer')}>
      <Box data-testid="traffic-tracer-workspace" sx={{ pb: 2 }}>
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
        {batches.batchStatus && (
          <Box sx={{ mb: 2 }}>
            <TrafficTracerBatchProgress
              status={batches.batchStatus}
              workspaceRoot={diagnosticRequest?.output_root ?? ''}
              cancelling={batches.cancelMutation.isPending}
              resuming={batches.resuming}
              onCancel={() =>
                void batches.cancelBatch(
                  'Cancelled from the TrafficTracer workspace.',
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
          onDiagnose={setDiagnosticRequest}
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
