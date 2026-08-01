import { Box } from '@mui/material'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'
import { TrafficTracerCaptureForm } from '@/components/traffic-tracer/capture-form'
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
  const { startCapture, startMutation } = useCaptureJob()

  const handleStartCapture = async (request: CaptureStartRequest) => {
    try {
      await startCapture(request)
      showNotice.success('TrafficTracer capture started.')
    } catch (error) {
      showNotice.error(error)
    }
  }

  return (
    <BasePage title={t('layout.components.navigation.tabs.trafficTracer')}>
      <Box data-testid="traffic-tracer-workspace" sx={{ pb: 2 }}>
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
      </Box>
    </BasePage>
  )
}

export default TrafficTracerPage
