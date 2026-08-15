import {
  FolderOpenRounded,
  ReplayRounded,
  VisibilityRounded,
  WarningAmberRounded,
} from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import type { JobState, SessionSummary } from '@/types/traffic-tracer'

export interface TrafficTracerSessionCardProps {
  session: SessionSummary
  opening?: boolean
  analyzing?: boolean
  analysisBlocked?: boolean
  onOpenDirectory: (sessionId: string) => void
  onAnalyze: (sessionId: string) => void
  onView: (sessionId: string) => void
}

const stateColor: Record<
  JobState,
  'default' | 'info' | 'success' | 'warning' | 'error'
> = {
  created: 'default',
  preparing: 'info',
  capturing: 'info',
  analyzing: 'info',
  completed: 'success',
  failed: 'error',
  cancelled: 'warning',
  interrupted: 'warning',
}

function formatDate(value: string) {
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

export function TrafficTracerSessionCard({
  session,
  opening = false,
  analyzing = false,
  analysisBlocked = false,
  onOpenDirectory,
  onAnalyze,
  onView,
}: TrafficTracerSessionCardProps) {
  const { t } = useTranslation()
  return (
    <Paper
      variant="outlined"
      data-testid={`traffic-tracer-session-${session.session_id}`}
      sx={{ p: 1.5 }}
    >
      <Stack spacing={1}>
        <Stack
          direction="row"
          spacing={1}
          sx={{ justifyContent: 'space-between', alignItems: 'flex-start' }}
        >
          <Box sx={{ minWidth: 0 }}>
            <Typography
              variant="subtitle1"
              title={session.target.url}
              sx={{
                fontWeight: 600,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {session.target.domain}
            </Typography>
            <Typography
              variant="body2"
              color="text.secondary"
              title={session.target.url}
              sx={{ overflow: 'hidden', textOverflow: 'ellipsis' }}
            >
              {session.target.url}
            </Typography>
          </Box>
          <Chip
            size="small"
            color={stateColor[session.state]}
            label={t(`settings.trafficTracer.common.states.${session.state}`)}
          />
        </Stack>

        <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
          <Chip
            size="small"
            variant="outlined"
            label={formatDate(session.created_at)}
          />
          <Chip
            size="small"
            variant="outlined"
            label={t(
              session.artifact_count === 1
                ? 'settings.trafficTracer.sessions.artifacts_one'
                : 'settings.trafficTracer.sessions.artifacts_other',
              {
                count: session.artifact_count,
              },
            )}
          />
          {session.packet_split?.status && (
            <Chip
              size="small"
              variant="outlined"
              color={
                session.packet_split.status.startsWith('complete')
                  ? 'success'
                  : ['partial', 'stale'].includes(session.packet_split.status)
                    ? 'warning'
                    : 'default'
              }
              label={t(
                `settings.trafficTracer.sessions.splitStatus.${session.packet_split.status}`,
              )}
            />
          )}
          {session.scenario_outcome_state &&
            session.scenario_outcome_state !== 'passed' && (
              <Chip
                size="small"
                color={
                  session.scenario_outcome_state === 'degraded'
                    ? 'warning'
                    : session.scenario_outcome_state === 'failed'
                      ? 'error'
                      : 'default'
                }
                label={`Playback: ${session.scenario_outcome_state}`}
              />
            )}
          {session.warning_count > 0 && (
            <Chip
              size="small"
              color="warning"
              icon={<WarningAmberRounded />}
              label={t(
                session.warning_count === 1
                  ? 'settings.trafficTracer.sessions.warnings_one'
                  : 'settings.trafficTracer.sessions.warnings_other',
                {
                  count: session.warning_count,
                },
              )}
            />
          )}
        </Stack>

        {session.error && (
          <Alert severity="error">
            {session.error.code}: {session.error.message}
          </Alert>
        )}

        <Stack direction="row" spacing={1} sx={{ justifyContent: 'flex-end' }}>
          <Button
            size="small"
            startIcon={<VisibilityRounded />}
            onClick={() => onView(session.session_id)}
          >
            {t('settings.trafficTracer.sessions.details')}
          </Button>
          <Button
            size="small"
            startIcon={<FolderOpenRounded />}
            disabled={opening}
            onClick={() => onOpenDirectory(session.session_id)}
          >
            {opening
              ? t('settings.trafficTracer.common.progress.opening')
              : t('settings.trafficTracer.common.actions.openDirectory')}
          </Button>
          <Button
            size="small"
            startIcon={<ReplayRounded />}
            disabled={analyzing || analysisBlocked}
            onClick={() => onAnalyze(session.session_id)}
          >
            {analyzing
              ? t('settings.trafficTracer.common.progress.starting')
              : t('settings.trafficTracer.common.actions.analyzeAgain')}
          </Button>
        </Stack>
      </Stack>
    </Paper>
  )
}
