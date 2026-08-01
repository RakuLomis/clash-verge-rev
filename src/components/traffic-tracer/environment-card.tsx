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

function errorMessage(error: unknown) {
  if (error instanceof Error) return error.message
  return typeof error === 'string' ? error : 'Environment diagnostics failed.'
}

export function TrafficTracerEnvironmentCard({
  report,
  request,
  loading = false,
  error,
  onRetry,
  onRemediate,
}: TrafficTracerEnvironmentCardProps) {
  const summary = buildEnvironmentSummary(report, request)
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
            Environment readiness
          </Typography>
          <Typography variant="body2" color="text.secondary">
            Core, capture tools, interfaces, browser, and Session storage
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
                  ? 'Ready'
                  : level === 'warning'
                    ? 'Needs attention'
                    : 'Blocked'
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
              Check again
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
                Retry
              </Button>
            ) : undefined
          }
          sx={{ borderRadius: 0 }}
        >
          {errorMessage(error)}
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
                      Fix
                    </Button>
                  ) : undefined
                }
              >
                <Typography variant="subtitle2">{check.message}</Typography>
                <Typography variant="body2">{check.remediation}</Typography>
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
