import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  Stack,
  Typography,
} from '@mui/material'
import { useTranslation } from 'react-i18next'

import { useTrafficTracerSession } from '@/hooks/use-traffic-tracer-sessions'
import { showNotice } from '@/services/notice-service'

import { TrafficTracerArtifactList } from './artifact-list'

export interface TrafficTracerSessionDetailProps {
  sessionId: string | null
  onClose: () => void
}

function versionLabel(version: string, commit: string) {
  return commit ? `${version} · ${commit.slice(0, 12)}` : version
}

export function TrafficTracerSessionDetail({
  sessionId,
  onClose,
}: TrafficTracerSessionDetailProps) {
  const { t } = useTranslation()
  const { session, sessionQuery, startAnalysis, analysisMutation } =
    useTrafficTracerSession(sessionId)

  const analyzeAgain = async () => {
    try {
      await startAnalysis({
        split_pcaps: true,
        write_flow_index: true,
        overwrite: true,
      })
      showNotice.success('settings.trafficTracer.notifications.analysisStarted')
    } catch (error) {
      showNotice.error(error)
    }
  }

  return (
    <Dialog open={sessionId !== null} onClose={onClose} fullWidth maxWidth="md">
      <DialogTitle>
        {t('settings.trafficTracer.sessions.detailTitle')}
      </DialogTitle>
      <DialogContent dividers>
        {sessionQuery.isLoading ? (
          <Stack sx={{ alignItems: 'center', py: 5 }}>
            <CircularProgress />
          </Stack>
        ) : sessionQuery.error ? (
          <Alert severity="error">{String(sessionQuery.error)}</Alert>
        ) : session ? (
          <Stack spacing={2}>
            <Box>
              <Typography variant="h6">{session.target.domain}</Typography>
              <Typography variant="body2" color="text.secondary">
                {session.target.url}
              </Typography>
            </Box>

            <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
              <Chip
                label={t(
                  `settings.trafficTracer.common.states.${session.state}`,
                )}
                size="small"
              />
              <Chip
                label={t('settings.trafficTracer.sessions.schema', {
                  version: session.schema_version,
                })}
                size="small"
              />
              <Chip
                label={t('settings.trafficTracer.sessions.workerApi', {
                  version: session.component_versions.worker_api,
                })}
                size="small"
              />
            </Stack>

            {session.error && (
              <Alert severity="error">
                {session.error.code}: {session.error.message}
                {session.error.stage ? ` (${session.error.stage})` : ''}
              </Alert>
            )}
            {session.warnings.map((warning) => (
              <Alert key={warning} severity="warning">
                {warning}
              </Alert>
            ))}

            <Box>
              <Typography variant="subtitle2" sx={{ mb: 1 }}>
                {t('settings.trafficTracer.sessions.componentVersions')}
              </Typography>
              <Stack spacing={0.75}>
                <Typography variant="body2">
                  TrafficTracer:{' '}
                  {versionLabel(
                    session.component_versions.traffictracer.version,
                    session.component_versions.traffictracer.commit,
                  )}
                </Typography>
                <Typography variant="body2">
                  Mihomo:{' '}
                  {versionLabel(
                    session.component_versions.mihomo.version,
                    session.component_versions.mihomo.commit,
                  )}
                </Typography>
                <Typography variant="body2">
                  Clash Verge:{' '}
                  {versionLabel(
                    session.component_versions.clash_verge_rev.version,
                    session.component_versions.clash_verge_rev.commit,
                  )}
                </Typography>
              </Stack>
            </Box>

            <Divider />
            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                {t('settings.trafficTracer.sessions.artifactsTitle')}
              </Typography>
              <TrafficTracerArtifactList
                sessionId={session.session_id}
                artifacts={session.artifacts}
              />
            </Box>
          </Stack>
        ) : null}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>
          {t('settings.trafficTracer.common.actions.close')}
        </Button>
        <Button
          variant="contained"
          disabled={!session || analysisMutation.isPending}
          onClick={() => void analyzeAgain()}
        >
          {analysisMutation.isPending
            ? t('settings.trafficTracer.sessions.startingAnalysis')
            : t('settings.trafficTracer.common.actions.analyzeAgain')}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
