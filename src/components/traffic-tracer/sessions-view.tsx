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

const PAGE_SIZE = 8

export function TrafficTracerSessionsView({
  enabled = true,
}: {
  enabled?: boolean
}) {
  const queryClient = useQueryClient()
  const [page, setPage] = useState(1)
  const [openingSessionId, setOpeningSessionId] = useState<string | null>(null)
  const { sessions, corrupt, total, sessionsQuery, refreshSessions } =
    useTrafficTracerSessions((page - 1) * PAGE_SIZE, PAGE_SIZE, enabled)

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
      showNotice.success('TrafficTracer analysis started.')
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
              Sessions
            </Typography>
            <Typography variant="body2" color="text.secondary">
              Captures and analysis results stored by TrafficTracer
            </Typography>
          </Box>
          <Button
            size="small"
            startIcon={<RefreshRounded />}
            disabled={!enabled || sessionsQuery.isFetching}
            onClick={() => void refreshSessions()}
          >
            Refresh
          </Button>
        </Stack>

        {corrupt.map((item) => (
          <Alert key={item.session_dir} severity="error">
            <Typography variant="subtitle2">Corrupt Session</Typography>
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
            Check the TrafficTracer environment to load Sessions.
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
                Retry
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
            No TrafficTracer Sessions yet.
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
    </Paper>
  )
}
