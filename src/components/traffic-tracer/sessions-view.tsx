import {
  ClearRounded,
  FolderOpenRounded,
  RefreshRounded,
} from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  Chip,
  Pagination,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { open } from '@tauri-apps/plugin-dialog'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { trafficTracerJobKey } from '@/hooks/use-capture-job'
import {
  trafficTracerSessionsKey,
  useTrafficTracerScopedSessions,
} from '@/hooks/use-traffic-tracer-sessions'
import {
  openTrafficTracerSessionDirectory,
  resolveTrafficTracerSessionScope,
  startTrafficTracerAnalysis,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type { SessionScope, SessionScopeSelector } from '@/types/traffic-tracer'

import { TrafficTracerSessionCard } from './session-card'
import { TrafficTracerSessionDetail } from './session-detail'

const PAGE_SIZE = 8

type ScopeSelection = {
  scope: SessionScope
  source: 'active' | 'manual'
}

export function TrafficTracerSessionsView({
  enabled = true,
  workspaceRoot = '',
  activeJobId = null,
  activeBatchId = null,
}: {
  enabled?: boolean
  workspaceRoot?: string
  activeJobId?: string | null
  activeBatchId?: string | null
}) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const activeKey = activeBatchId
    ? `batch:${activeBatchId}`
    : activeJobId
      ? `job:${activeJobId}`
      : null
  const [manualSelection, setManualSelection] = useState<{
    scope: SessionScope
    activeKey: string | null
    workspaceRoot: string
  } | null>(null)
  const [suppressedActiveKey, setSuppressedActiveKey] = useState<string | null>(
    null,
  )
  const [pageState, setPageState] = useState<{
    scopeId: string | null
    page: number
  }>({ scopeId: null, page: 1 })
  const [selectedSession, setSelectedSession] = useState<{
    scopeId: string
    sessionId: string
  } | null>(null)
  const [openingSessionId, setOpeningSessionId] = useState<string | null>(null)

  const activeSelector = useMemo<SessionScopeSelector | null>(() => {
    if (activeBatchId) return { batch_id: activeBatchId }
    if (activeJobId) return { job_id: activeJobId }
    return null
  }, [activeBatchId, activeJobId])

  const activeScopeQuery = useQuery({
    queryKey: ['trafficTracer', 'sessionScope', workspaceRoot, activeKey],
    queryFn: () => resolveTrafficTracerSessionScope(activeSelector!),
    enabled: enabled && activeSelector !== null,
    refetchInterval: ({ state }) => (state.data ? false : 1000),
  })

  const applicableManualSelection =
    manualSelection &&
    manualSelection.workspaceRoot === workspaceRoot &&
    (!activeKey || manualSelection.activeKey === activeKey)
      ? manualSelection
      : null
  const activeSelection =
    activeKey && activeScopeQuery.data && suppressedActiveKey !== activeKey
      ? ({ scope: activeScopeQuery.data, source: 'active' } as const)
      : null
  const selection: ScopeSelection | null = applicableManualSelection
    ? { scope: applicableManualSelection.scope, source: 'manual' }
    : activeSelection
  const scopeId = selection?.scope.scope_id ?? null
  const page = pageState.scopeId === scopeId ? pageState.page : 1
  const selectedSessionId =
    selectedSession?.scopeId === scopeId ? selectedSession.sessionId : null
  const { sessions, corrupt, total, sessionsQuery, refreshSessions } =
    useTrafficTracerScopedSessions(
      scopeId,
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

  const chooseScope = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        defaultPath: workspaceRoot || undefined,
      })
      if (typeof selected !== 'string') return
      const scope = await resolveTrafficTracerSessionScope({ path: selected })
      if (!scope) throw new Error('Selected Session scope was not found')
      setManualSelection({ scope, activeKey, workspaceRoot })
      setPageState({ scopeId: scope.scope_id, page: 1 })
      setSelectedSession(null)
    } catch (error) {
      showNotice.error(error)
    }
  }

  const clearScope = () => {
    setManualSelection(null)
    setSuppressedActiveKey(activeKey)
    setPageState({ scopeId: null, page: 1 })
    setSelectedSession(null)
  }

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
          direction={{ xs: 'column', sm: 'row' }}
          spacing={1}
          sx={{ justifyContent: 'space-between', alignItems: { sm: 'center' } }}
        >
          <Box>
            <Typography variant="h6" sx={{ fontSize: 17, fontWeight: 600 }}>
              {t('settings.trafficTracer.sessions.title')}
            </Typography>
            <Typography variant="body2" color="text.secondary">
              {t('settings.trafficTracer.sessions.description')}
            </Typography>
          </Box>
          <Stack direction="row" spacing={1} sx={{ flexWrap: 'wrap' }}>
            <Button
              size="small"
              startIcon={<FolderOpenRounded />}
              disabled={!enabled || !workspaceRoot}
              onClick={() => void chooseScope()}
            >
              {t('settings.trafficTracer.sessions.chooseFolder')}
            </Button>
            {selection && (
              <Button
                size="small"
                startIcon={<ClearRounded />}
                onClick={clearScope}
              >
                {t('settings.trafficTracer.sessions.clearFolder')}
              </Button>
            )}
            <Button
              size="small"
              startIcon={<RefreshRounded />}
              disabled={!enabled || !scopeId || sessionsQuery.isFetching}
              onClick={() => void refreshSessions()}
            >
              {t('settings.trafficTracer.common.actions.refresh')}
            </Button>
          </Stack>
        </Stack>

        {selection && (
          <Paper variant="outlined" sx={{ px: 1.5, py: 1 }}>
            <Stack
              direction={{ xs: 'column', sm: 'row' }}
              spacing={1}
              sx={{ alignItems: { sm: 'center' } }}
            >
              <Typography variant="body2" sx={{ fontWeight: 600 }}>
                {selection.scope.display_name}
              </Typography>
              {selection.source === 'active' && (
                <Chip
                  size="small"
                  color="primary"
                  label={t('settings.trafficTracer.sessions.currentCapture')}
                />
              )}
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ overflowWrap: 'anywhere' }}
              >
                {selection.scope.directory}
              </Typography>
            </Stack>
          </Paper>
        )}

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
        ) : activeScopeQuery.error && !selection ? (
          <Alert severity="error">{String(activeScopeQuery.error)}</Alert>
        ) : !selection ? (
          <Typography
            color="text.secondary"
            sx={{ py: 4, textAlign: 'center' }}
          >
            {activeKey && activeScopeQuery.isFetching
              ? t('settings.trafficTracer.sessions.waitingForCaptureFolder')
              : t('settings.trafficTracer.sessions.noFolderSelected')}
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
            {selection.scope.exists
              ? t('settings.trafficTracer.sessions.emptyFolder')
              : t('settings.trafficTracer.sessions.waitingForCaptureFolder')}
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
                onView={(sessionId) => {
                  if (scopeId) setSelectedSession({ scopeId, sessionId })
                }}
              />
            ))}
          </Stack>
        )}

        {selection && totalPages > 1 && (
          <Pagination
            page={Math.min(page, totalPages)}
            count={totalPages}
            onChange={(_, nextPage) =>
              setPageState({ scopeId, page: nextPage })
            }
            sx={{ alignSelf: 'center' }}
          />
        )}
      </Stack>
      <TrafficTracerSessionDetail
        sessionId={selectedSessionId}
        workspaceRoot={workspaceRoot}
        onClose={() => setSelectedSession(null)}
      />
    </Paper>
  )
}
