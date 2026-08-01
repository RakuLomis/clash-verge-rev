import { Box } from '@mui/material'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import { TrafficTracerCaptureForm } from '@/components/traffic-tracer/capture-form'
import { TrafficTracerFlowQueryForm } from '@/components/traffic-tracer/flow-query-form'
import { TrafficTracerJobProgress } from '@/components/traffic-tracer/job-progress'
import { TrafficTracerSessionsView } from '@/components/traffic-tracer/sessions-view'
import { useCaptureJob } from '@/hooks/use-capture-job'
import { useTrafficTracerWorker } from '@/hooks/use-traffic-tracer-worker'
import { showNotice } from '@/services/notice-service'
import type {
  CaptureStartRequest,
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

  const handleStartCapture = async (request: CaptureStartRequest) => {
    try {
      await startCapture(request)
      showNotice.success('TrafficTracer capture started.')
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
        <TrafficTracerCaptureForm
          environment={environment}
          diagnosticRequest={diagnosticRequest}
          diagnosing={environmentQuery.isFetching}
          diagnosticError={environmentQuery.error}
          captureLocked={captureLock?.locked}
          submitting={startMutation.isPending}
          onDiagnose={setDiagnosticRequest}
          onRetryDiagnostics={() => void environmentQuery.refetch()}
          onSubmit={handleStartCapture}
        />
        <Box sx={{ mt: 2 }}>
          <TrafficTracerSessionsView enabled={environment !== undefined} />
        </Box>
        <Box sx={{ mt: 2 }}>
          <TrafficTracerFlowQueryForm enabled={environment !== undefined} />
        </Box>
      </Box>
    </BasePage>
  )
}

export default TrafficTracerPage
