import { RefreshRounded } from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Pagination,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'

import { trafficTracerJobKey } from '@/hooks/use-capture-job'
import {
  trafficTracerSessionsKey,
  useTrafficTracerSessions,
} from '@/hooks/use-traffic-tracer-sessions'
import {
  openTrafficTracerSessionDirectory,
  startTrafficTracerAnalysis,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'

import { TrafficTracerSessionCard } from './session-card'
import { TrafficTracerSessionDetail } from './session-detail'

const PAGE_SIZE = 8

export function TrafficTracerSessionsView({
  enabled = true,
  workspaceRoot = '',
}: {
  enabled?: boolean
  workspaceRoot?: string
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [page, setPage] = useState(1)
  const [selectedSessionId, setSelectedSessionId] = useState<string | null>(
    null,
  )
  const [openingSessionId, setOpeningSessionId] = useState<string | null>(null)
  const { sessions, corrupt, total, sessionsQuery, refreshSessions } =
    useTrafficTracerSessions(
      (page - 1) * PAGE_SIZE,
      PAGE_SIZE,
      enabled,
      workspaceRoot,
    )

  const analysisMutation = useMutation({
    mutationFn: (sessionId: string) =>
      startTrafficTracerAnalysis(sessionId, {
        split_pcaps: true,
        write_flow_index: true,
        overwrite: true,
      }),
    onSuccess: (snapshot) => {
      queryClient.setQueryData(trafficTracerJobKey(snapshot.job_id), snapshot)
      void queryClient.invalidateQueries({ queryKey: trafficTracerSessionsKey })
      showNotice.success('settings.trafficTracer.notifications.analysisStarted')
    },
    onError: (error) => showNotice.error(error),
  })

  const openDirectory = async (sessionId: string) => {
    try {
      setOpeningSessionId(sessionId)
      await openTrafficTracerSessionDirectory(sessionId)
    } catch (error) {
      showNotice.error(error)
    } finally {
      setOpeningSessionId(null)
    }
  }

  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE))

  return (
    <Paper
      variant="outlined"
      sx={{ p: 2 }}
      data-testid="traffic-tracer-sessions"
    >
      <Stack spacing={1.5}>
        <Stack
          direction="row"
          spacing={1}
          sx={{ justifyContent: 'space-between', alignItems: 'center' }}
        >
          <Box>
            <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
              {t('settings.trafficTracer.sessions.title')}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {t('settings.trafficTracer.sessions.description')}
            </Typography>
          </Box>
          <Button
            size="small"
            startIcon={<RefreshRounded />}
            disabled={!enabled || sessionsQuery.isFetching}
            onClick={() => void refreshSessions()}
          >
            {t('settings.trafficTracer.common.actions.refresh')}
          </Button>
        </Stack>

        {corrupt.map((item) => (
          <Alert key={item.session_dir} severity="error">
            <Typography variant="subtitle2">
              {t('settings.trafficTracer.sessions.corrupt')}
            </Typography>
            <Typography variant="body2">{item.message}</Typography>
            <Typography variant="caption" sx={{ overflowWrap: 'anywhere' }}>
              {item.session_dir}
            </Typography>
          </Alert>
        ))}

        {!enabled ? (
          <Typography
            color="text.secondary"
            sx={{ py: 4, textAlign: 'center' }}
          >
            {t('settings.trafficTracer.sessions.checkEnvironment')}
          </Typography>
        ) : sessionsQuery.isLoading ? (
          <Stack sx={{ alignItems: 'center', py: 4 }}>
            <CircularProgress size={28} />
          </Stack>
        ) : sessionsQuery.error ? (
          <Alert
            severity="error"
            action={
              <Button
                color="inherit"
                size="small"
                onClick={() => void refreshSessions()}
              >
                {t('settings.trafficTracer.common.actions.retry')}
              </Button>
            }
          >
            {String(sessionsQuery.error)}
          </Alert>
        ) : total === 0 ? (
          <Typography
            color="text.secondary"
            sx={{ py: 4, textAlign: 'center' }}
          >
            {t('settings.trafficTracer.sessions.empty')}
          </Typography>
        ) : (
          <Stack spacing={1}>
            {sessions.map((session) => (
              <TrafficTracerSessionCard
                key={session.session_id}
                session={session}
                opening={openingSessionId === session.session_id}
                analyzing={
                  analysisMutation.isPending &&
                  analysisMutation.variables === session.session_id
                }
                analysisBlocked={
                  analysisMutation.isPending &&
                  analysisMutation.variables !== session.session_id
                }
                onOpenDirectory={(sessionId) => void openDirectory(sessionId)}
                onAnalyze={(sessionId) => analysisMutation.mutate(sessionId)}
                onView={setSelectedSessionId}
              />
            ))}
          </Stack>
        )}

        {totalPages > 1 && (
          <Pagination
            page={Math.min(page, totalPages)}
            count={totalPages}
            onChange={(_, nextPage) => setPage(nextPage)}
            sx={{ alignSelf: 'center' }}
          />
        )}
      </Stack>
      <TrafficTracerSessionDetail
        sessionId={selectedSessionId}
        workspaceRoot={workspaceRoot}
        onClose={() => setSelectedSessionId(null)}
      />
    </Paper>
  )
}
