import { Box } from '@mui/material'
import { useTranslation } from 'react-i18next'

import { BasePage } from '@/components/base'

const TrafficTracerPage = () => {
  const { t } = useTranslation()

  return (
    <BasePage title={t('layout.components.navigation.tabs.trafficTracer')}>
      <Box data-testid="traffic-tracer-workspace" />
    </BasePage>
  )
}

export default TrafficTracerPage
