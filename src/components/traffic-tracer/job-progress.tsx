import {
  CancelRounded,
  CheckCircleRounded,
  ErrorRounded,
  HourglassTopRounded,
} from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  LinearProgress,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import type {
  JobProgressEvent,
  JobSnapshot,
  JobState,
} from '@/types/traffic-tracer'

export interface TrafficTracerJobProgressProps {
  job: JobSnapshot
  startedAt?: string | null
  events?: JobProgressEvent[]
  cancelling?: boolean
  onCancel: (reason?: string) => Promise<unknown> | void
}

const emptyProgressEvents: JobProgressEvent[] = []

const terminalStates = new Set<JobState>([
  'completed',
  'failed',
  'cancelled',
  'interrupted',
])

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

function formatTime(value: string | null | undefined, restoredTime: string) {
  if (!value) return restoredTime
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString()
}

function errorText(error: unknown) {
  if (typeof error === 'string') return error
  if (typeof error !== 'object' || error === null) return ''
  const payload = error as Record<string, unknown>
  const code = typeof payload.code === 'string' ? payload.code : ''
  const message = typeof payload.message === 'string' ? payload.message : ''
  return [code, message].filter(Boolean).join(': ')
}

function trafficTracerOperationLabel(operation: string) {
  if (operation.startsWith('catalog.')) return 'Session catalog migration'
  if (operation === 'worker.recovery') return 'Interrupted Session recovery'
  if (operation.includes('tshark')) return 'Packet capture startup'
  if (operation === 'capture.chrome_launch') return 'Chrome startup'
  if (operation === 'capture.cdp_connect') return 'Chrome DevTools connection'
  if (operation === 'capture.navigation') return 'Page navigation'
  if (operation === 'capture.observation') return 'Page observation'
  if (operation.startsWith('analyze.')) return 'Traffic analysis'
  if (operation.includes('split')) return 'Packet splitting'
  if (operation === 'capture.cleanup') return 'Capture cleanup'
  if (operation.startsWith('core.')) return 'Mihomo preparation'
  if (operation === 'capture.prepare_paths') return 'Session preparation'
  return operation || 'Preparing'
}

function formatDuration(milliseconds: number) {
  const seconds = Math.max(0, milliseconds) / 1000
  return seconds < 10 ? `${seconds.toFixed(1)}s` : `${Math.round(seconds)}s`
}

export function TrafficTracerJobProgress({
  job,
  startedAt,
  events = emptyProgressEvents,
  cancelling = false,
  onCancel,
}: TrafficTracerJobProgressProps) {
  const { t } = useTranslation()
  const [confirmOpen, setConfirmOpen] = useState(false)
  const terminal = terminalStates.has(job.state)
  const canceling = cancelling || job.cancel_requested
  const progress = Math.round(
    Math.min(1, Math.max(0, Number.isFinite(job.progress) ? job.progress : 0)) *
      100,
  )
  const failure = errorText(job.error)
  const logEvents = events.filter(
    (event) => event.message || event.stage || event.state,
  )
  const [now, setNow] = useState(() => Date.now())
  const latestEvent = events.at(-1)
  useEffect(() => {
    if (terminal || !latestEvent?.timing) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [latestEvent?.timing, terminal])
  const liveTiming = useMemo(() => {
    if (!latestEvent?.timing) return null
    const eventTime = new Date(latestEvent.timestamp).getTime()
    const sinceEvent =
      terminal || Number.isNaN(eventTime) ? 0 : Math.max(0, now - eventTime)
    return {
      ...latestEvent.timing,
      job_elapsed_ms: latestEvent.timing.job_elapsed_ms + sinceEvent,
      stage_elapsed_ms: latestEvent.timing.stage_elapsed_ms + sinceEvent,
      operation_elapsed_ms:
        latestEvent.timing.operation_elapsed_ms + sinceEvent,
    }
  }, [latestEvent, now, terminal])

  const confirmCancel = async () => {
    setConfirmOpen(false)
    await onCancel(t('settings.trafficTracer.jobs.cancelReason'))
  }

  return (
    <>
      <Paper
        variant="outlined"
        data-testid="traffic-tracer-job-progress"
        data-job-state={job.state}
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
          <Stack direction="row" spacing={1.5} sx={{ alignItems: 'center' }}>
            {job.state === 'completed' ? (
              <CheckCircleRounded color="success" />
            ) : job.state === 'failed' ? (
              <ErrorRounded color="error" />
            ) : (
              <HourglassTopRounded color={terminal ? 'warning' : 'primary'} />
            )}
            <Box>
              <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
                {job.kind === 'capture'
                  ? t('settings.trafficTracer.jobs.capture')
                  : t('settings.trafficTracer.jobs.analysis')}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {t('settings.trafficTracer.jobs.started', {
                  time: formatTime(
                    startedAt,
                    t('settings.trafficTracer.jobs.restoredTime'),
                  ),
                })}
              </Typography>
            </Box>
          </Stack>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Chip
              size="small"
              color={stateColor[job.state]}
              label={t(`settings.trafficTracer.common.states.${job.state}`)}
            />
            {!terminal && (
              <Button
                color="error"
                size="small"
                startIcon={<CancelRounded />}
                disabled={canceling}
                onClick={() => setConfirmOpen(true)}
              >
                {canceling
                  ? t('settings.trafficTracer.common.progress.canceling')
                  : t('settings.trafficTracer.common.actions.cancel')}
              </Button>
            )}
          </Stack>
        </Stack>

        <Divider />

        <Stack spacing={1.5} sx={{ p: 2 }}>
          <Stack
            direction="row"
            sx={{ justifyContent: 'space-between', alignItems: 'baseline' }}
          >
            <Typography variant="body2" sx={{ fontWeight: 600 }}>
              {job.stage}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {progress}%
            </Typography>
          </Stack>
          <LinearProgress
            variant="determinate"
            value={progress}
            color={job.state === 'failed' ? 'error' : 'primary'}
          />
          {job.message && (
            <Typography variant="body2">{job.message}</Typography>
          )}
          {liveTiming && (
            <Alert severity="info" data-testid="traffic-tracer-job-timing">
              {trafficTracerOperationLabel(liveTiming.operation)} · operation{' '}
              {formatDuration(liveTiming.operation_elapsed_ms)} · stage{' '}
              {formatDuration(liveTiming.stage_elapsed_ms)} · total{' '}
              {formatDuration(liveTiming.job_elapsed_ms)}
            </Alert>
          )}
          {failure && (
            <Alert severity={job.state === 'failed' ? 'error' : 'warning'}>
              {failure}
            </Alert>
          )}

          <Box>
            <Typography variant="subtitle2" sx={{ mb: 0.75 }}>
              {t('settings.trafficTracer.jobs.progressLog')}
            </Typography>
            <Paper
              variant="outlined"
              sx={{ maxHeight: 180, overflow: 'auto', bgcolor: 'action.hover' }}
            >
              {logEvents.length === 0 ? (
                <Typography
                  variant="body2"
                  color="text.secondary"
                  sx={{ p: 1.5 }}
                >
                  {t('settings.trafficTracer.jobs.waiting')}
                </Typography>
              ) : (
                <Stack divider={<Divider flexItem />}>
                  {logEvents.map((event) => (
                    <Stack
                      key={
                        event.timestamp +
                        event.stage +
                        event.progress +
                        event.message
                      }
                      direction="row"
                      spacing={1}
                      sx={{ px: 1.5, py: 0.75, alignItems: 'baseline' }}
                    >
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        sx={{ flexShrink: 0 }}
                      >
                        {new Date(event.timestamp).toLocaleTimeString()}
                      </Typography>
                      <Typography variant="caption" sx={{ fontWeight: 600 }}>
                        {event.stage}
                      </Typography>
                      <Typography variant="caption">
                        {event.message || event.state}
                      </Typography>
                      {event.timing && (
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ ml: 'auto !important', flexShrink: 0 }}
                        >
                          {trafficTracerOperationLabel(event.timing.operation)}{' '}
                          · {formatDuration(event.timing.operation_elapsed_ms)}
                        </Typography>
                      )}
                    </Stack>
                  ))}
                </Stack>
              )}
            </Paper>
          </Box>

          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ fontFamily: 'monospace', overflowWrap: 'anywhere' }}
          >
            {t('settings.trafficTracer.jobs.jobId', { id: job.job_id })}
          </Typography>
        </Stack>
      </Paper>

      <Dialog open={confirmOpen} onClose={() => setConfirmOpen(false)}>
        <DialogTitle>
          {t('settings.trafficTracer.jobs.cancelTitle')}
        </DialogTitle>
        <DialogContent>
          <DialogContentText>
            {t('settings.trafficTracer.jobs.cancelDescription')}
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setConfirmOpen(false)}>
            {t('settings.trafficTracer.common.actions.keepRunning')}
          </Button>
          <Button
            color="error"
            variant="contained"
            onClick={() => void confirmCancel()}
          >
            {t('settings.trafficTracer.common.actions.cancelJob')}
          </Button>
        </DialogActions>
      </Dialog>
    </>
  )
}
