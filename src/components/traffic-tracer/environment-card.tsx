import {
  BuildRounded,
  CheckCircleRounded,
  ErrorRounded,
  FactCheckRounded,
  RefreshRounded,
  WarningRounded,
} from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Divider,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'

import type { TranslationKey } from '@/types/generated/i18n-keys'

import type {
  EnvironmentItemState,
  TrafficTracerEnvironmentCardProps,
} from './environment-model'
import {
  buildEnvironmentSummary,
  remediationTargetFor,
} from './environment-model'

const statePresentation: Record<EnvironmentItemState, { icon: ReactNode }> = {
  ready: {
    icon: <CheckCircleRounded fontSize="small" color="success" />,
  },
  warning: {
    icon: <WarningRounded fontSize="small" color="warning" />,
  },
  error: {
    icon: <ErrorRounded fontSize="small" color="error" />,
  },
  unknown: {
    icon: <FactCheckRounded fontSize="small" color="disabled" />,
  },
}

function errorMessage(error: unknown, fallback: string) {
  if (error instanceof Error) return error.message
  return typeof error === 'string' ? error : fallback
}

export function TrafficTracerEnvironmentCard({
  report,
  request,
  loading = false,
  error,
  onRetry,
  onRemediate,
}: TrafficTracerEnvironmentCardProps) {
  const { t } = useTranslation()
  const summary = buildEnvironmentSummary(report, request, {
    labels: {
      core: t('settings.trafficTracer.environment.labels.core'),
      controller: t('settings.trafficTracer.environment.labels.controller'),
      tun: t('settings.trafficTracer.environment.labels.tun'),
      'tun-interface': t(
        'settings.trafficTracer.environment.labels.tunInterface',
      ),
      'physical-interface': t(
        'settings.trafficTracer.environment.labels.physicalInterface',
      ),
      'capture-tools': t(
        'settings.trafficTracer.environment.labels.captureTools',
      ),
      chrome: t('settings.trafficTracer.environment.labels.chrome'),
      output: t('settings.trafficTracer.environment.labels.output'),
    },
    localController: t(
      'settings.trafficTracer.environment.values.localController',
    ),
    notChecked: t('settings.trafficTracer.common.states.notChecked'),
    notSelected: t('settings.trafficTracer.environment.values.notSelected'),
    disabled: t('settings.trafficTracer.common.states.disabled'),
    tunServiceReady: t(
      'settings.trafficTracer.environment.values.tunServiceReady',
    ),
    tunServiceUnavailable: t(
      'settings.trafficTracer.environment.values.tunServiceUnavailable',
    ),
  })
  const failingChecks = report?.checks.filter((check) => !check.ok) ?? []
  const level = report?.level ?? 'blocking'
  const levelColor =
    level === 'ready' ? 'success' : level === 'warning' ? 'warning' : 'error'

  return (
    <Paper
      variant="outlined"
      data-testid="traffic-tracer-environment-card"
      data-environment-level={report?.level ?? 'unknown'}
      sx={{ overflow: 'hidden' }}
    >
      <Stack
        direction="row"
        spacing={2}
        sx={{
          alignItems: 'center',
          justifyContent: 'space-between',
          px: 2,
          py: 1.5,
        }}
      >
        <Box>
          <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
            {t('settings.trafficTracer.environment.title')}
          </Typography>
          <Typography variant="body2" color="text.secondary">
            {t('settings.trafficTracer.environment.description')}
          </Typography>
        </Box>
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
          {loading && <CircularProgress size={20} />}
          {report && (
            <Chip
              size="small"
              color={levelColor}
              label={
                level === 'ready'
                  ? t('settings.trafficTracer.common.states.ready')
                  : level === 'warning'
                    ? t('settings.trafficTracer.common.states.warning')
                    : t('settings.trafficTracer.common.states.blocked')
              }
            />
          )}
          {onRetry && (
            <Button
              size="small"
              startIcon={<RefreshRounded />}
              onClick={onRetry}
              disabled={loading}
            >
              {t('settings.trafficTracer.environment.checkAgain')}
            </Button>
          )}
        </Stack>
      </Stack>

      <Divider />

      <Box
        sx={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(220px, 1fr))',
          gap: 1,
          p: 2,
        }}
      >
        {summary.map((item) => (
          <Stack
            key={item.id}
            direction="row"
            spacing={1}
            data-testid={`environment-item-${item.id}`}
            data-state={item.state}
            sx={{
              alignItems: 'flex-start',
              minWidth: 0,
              p: 1,
              borderRadius: 1,
              bgcolor: 'action.hover',
            }}
          >
            {statePresentation[item.state].icon}
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="caption" color="text.secondary">
                {item.label}
              </Typography>
              <Typography
                variant="body2"
                title={item.value}
                sx={{ overflow: 'hidden', textOverflow: 'ellipsis' }}
              >
                {item.value}
              </Typography>
            </Box>
          </Stack>
        ))}
      </Box>

      {(error != null || failingChecks.length > 0) && <Divider />}

      {error != null && (
        <Alert
          severity="error"
          action={
            onRetry ? (
              <Button color="inherit" size="small" onClick={onRetry}>
                {t('settings.trafficTracer.environment.actions.retry')}
              </Button>
            ) : undefined
          }
          sx={{ borderRadius: 0 }}
        >
          {errorMessage(
            error,
            t('settings.trafficTracer.environment.diagnosticsFailed'),
          )}
        </Alert>
      )}

      {failingChecks.length > 0 && (
        <Stack spacing={1} sx={{ p: 2 }} data-testid="environment-remediations">
          {failingChecks.map((check) => {
            const target = remediationTargetFor(check.code)
            return (
              <Alert
                key={check.code}
                severity={check.severity === 'info' ? 'info' : check.severity}
                icon={false}
                action={
                  onRemediate ? (
                    <Button
                      color="inherit"
                      size="small"
                      startIcon={<BuildRounded />}
                      onClick={() => onRemediate(target, check)}
                    >
                      {t('settings.trafficTracer.environment.actions.fix')}
                    </Button>
                  ) : undefined
                }
              >
                <Typography variant="subtitle2">
                  {t(
                    `settings.trafficTracer.environment.diagnostics.${check.code}.message` as TranslationKey,
                    { defaultValue: check.message },
                  )}
                </Typography>
                <Typography variant="body2">
                  {t(
                    `settings.trafficTracer.environment.diagnostics.${check.code}.remediation` as TranslationKey,
                    { defaultValue: check.remediation },
                  )}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {check.code}
                </Typography>
              </Alert>
            )
          })}
        </Stack>
      )}
    </Paper>
  )
}
