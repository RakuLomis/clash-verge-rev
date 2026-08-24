import {
  CallSplitRounded,
  ClearRounded,
  FolderOpenRounded,
  RefreshRounded,
} from '@mui/icons-material'
import {
  Alert,
  Box,
  Button,
  CircularProgress,
  LinearProgress,
  Chip,
  Pagination,
  Paper,
  Stack,
  Typography,
} from '@mui/material'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { open } from '@tauri-apps/plugin-dialog'
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'

import { trafficTracerJobKey } from '@/hooks/use-capture-job'
import {
  trafficTracerSessionsKey,
  useTrafficTracerScopedSessions,
} from '@/hooks/use-traffic-tracer-sessions'
import {
  cancelTrafficTracerJob,
  getTrafficTracerJob,
  openTrafficTracerSessionDirectory,
  previewTrafficTracerPacketSplit,
  resolveTrafficTracerSessionScope,
  startTrafficTracerAnalysis,
  startTrafficTracerPacketSplit,
} from '@/services/cmds'
import { showNotice } from '@/services/notice-service'
import type { SessionScope, SessionScopeSelector } from '@/types/traffic-tracer'

import { TrafficTracerSessionCard } from './session-card'
import { TrafficTracerSessionDetail } from './session-detail'

const PAGE_SIZE = 8
const SESSION_WORKSPACE_KEY = 'traffictracer.sessionWorkspace.v1'

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
  const workspaceStorageKey = `${SESSION_WORKSPACE_KEY}:${encodeURIComponent(
    workspaceRoot,
  )}`
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
  const [splitJobId, setSplitJobId] = useState<string | null>(() =>
    localStorage.getItem('traffictracer.packetSplitJobId'),
  )

  const persistManualWorkspace = useCallback(
    (path: string, page: number, sessionId: string | null) => {
      localStorage.setItem(
        workspaceStorageKey,
        JSON.stringify({
          workspace_root: workspaceRoot,
          path,
          page,
          session_id: sessionId,
        }),
      )
    },
    [workspaceRoot, workspaceStorageKey],
  )

  useEffect(() => {
    if (!enabled || !workspaceRoot || activeKey) return
    const stored = localStorage.getItem(workspaceStorageKey)
    if (!stored) return
    let payload: {
      workspace_root?: unknown
      path?: unknown
      page?: unknown
      session_id?: unknown
    }
    try {
      payload = JSON.parse(stored)
    } catch {
      localStorage.removeItem(workspaceStorageKey)
      return
    }
    if (
      payload.workspace_root !== workspaceRoot ||
      typeof payload.path !== 'string' ||
      typeof payload.page !== 'number' ||
      !Number.isInteger(payload.page) ||
      payload.page < 1 ||
      !(payload.session_id === null || typeof payload.session_id === 'string')
    ) {
      localStorage.removeItem(workspaceStorageKey)
      return
    }
    let disposed = false
    void resolveTrafficTracerSessionScope({ path: payload.path })
      .then((scope) => {
        if (disposed || !scope) return
        setManualSelection({ scope, activeKey: null, workspaceRoot })
        setPageState({ scopeId: scope.scope_id, page: payload.page as number })
        setSelectedSession(
          typeof payload.session_id === 'string'
            ? {
                scopeId: scope.scope_id,
                sessionId: payload.session_id,
              }
            : null,
        )
      })
      .catch(() => {
        if (!disposed) localStorage.removeItem(workspaceStorageKey)
      })
    return () => {
      disposed = true
    }
  }, [activeKey, enabled, workspaceRoot, workspaceStorageKey])

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

  const splitPreviewQuery = useQuery({
    queryKey: ['trafficTracer', 'packetSplitPreview', workspaceRoot, scopeId],
    queryFn: () => previewTrafficTracerPacketSplit(scopeId!),
    enabled:
      enabled &&
      activeBatchId === null &&
      scopeId !== null &&
      selection?.scope.kind === 'capture_group',
    retry: 1,
  })
  const splitJobQuery = useQuery({
    queryKey: splitJobId
      ? trafficTracerJobKey(splitJobId)
      : ['trafficTracer', 'job', 'packetSplitNone'],
    queryFn: () => getTrafficTracerJob(splitJobId!),
    enabled: splitJobId !== null,
    retry: false,
    refetchInterval: ({ state }) =>
      state.data &&
      !['completed', 'failed', 'cancelled', 'interrupted'].includes(
        state.data.state,
      )
        ? 1000
        : false,
  })
  const splitMutation = useMutation({
    mutationFn: (policy: 'missing_only' | 'repair_incomplete') => {
      if (!scopeId) throw new Error('No timestamp capture folder is selected')
      return startTrafficTracerPacketSplit(scopeId, policy)
    },
    onSuccess: (snapshot) => {
      localStorage.setItem('traffictracer.packetSplitJobId', snapshot.job_id)
      setSplitJobId(snapshot.job_id)
      queryClient.setQueryData(trafficTracerJobKey(snapshot.job_id), snapshot)
    },
    onError: (error) => showNotice.error(error),
  })
  const splitCancelMutation = useMutation({
    mutationFn: () => {
      if (!splitJobId) throw new Error('No packet split Job is active')
      return cancelTrafficTracerJob(
        splitJobId,
        'Cancelled from Sessions packet split.',
      )
    },
    onSuccess: (snapshot) =>
      queryClient.setQueryData(trafficTracerJobKey(snapshot.job_id), snapshot),
    onError: (error) => showNotice.error(error),
  })

  useEffect(() => {
    if (!splitJobQuery.isError) return
    localStorage.removeItem('traffictracer.packetSplitJobId')
  }, [splitJobQuery.isError])

  const splitJob = splitJobQuery.data
  const splitJobActive = Boolean(
    splitJob &&
      !['completed', 'failed', 'cancelled', 'interrupted'].includes(
        splitJob.state,
      ),
  )

  useEffect(() => {
    if (
      !splitJob ||
      !['completed', 'failed', 'cancelled', 'interrupted'].includes(
        splitJob.state,
      )
    )
      return
    localStorage.removeItem('traffictracer.packetSplitJobId')
    void queryClient.invalidateQueries({ queryKey: trafficTracerSessionsKey })
    void queryClient.invalidateQueries({
      queryKey: ['trafficTracer', 'packetSplitPreview', workspaceRoot, scopeId],
    })
  }, [queryClient, scopeId, splitJob, workspaceRoot])

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
      persistManualWorkspace(scope.directory, 1, null)
    } catch (error) {
      showNotice.error(error)
    }
  }

  const clearScope = () => {
    localStorage.removeItem(workspaceStorageKey)
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
            {selection.scope.kind === 'capture_group' && (
              <Stack
                direction={{ xs: 'column', sm: 'row' }}
                spacing={1}
                sx={{ mt: 1, alignItems: { sm: 'center' } }}
              >
                <Typography variant="caption" color="text.secondary">
                  {splitPreviewQuery.isLoading
                    ? t('settings.trafficTracer.sessions.splitChecking')
                    : t('settings.trafficTracer.sessions.splitSummary', {
                        missing: splitPreviewQuery.data?.missing_only ?? 0,
                        repair: splitPreviewQuery.data?.repair_incomplete ?? 0,
                        complete:
                          (splitPreviewQuery.data?.counts.complete ?? 0) +
                          (splitPreviewQuery.data?.counts.complete_empty ?? 0),
                      })}
                </Typography>
                <Button
                  size="small"
                  variant="contained"
                  startIcon={<CallSplitRounded />}
                  disabled={
                    splitJobActive ||
                    splitMutation.isPending ||
                    !splitPreviewQuery.data?.missing_only
                  }
                  onClick={() => splitMutation.mutate('missing_only')}
                >
                  {t('settings.trafficTracer.sessions.splitMissing', {
                    count: splitPreviewQuery.data?.missing_only ?? 0,
                  })}
                </Button>
                <Button
                  size="small"
                  variant="outlined"
                  disabled={
                    splitJobActive ||
                    splitMutation.isPending ||
                    !splitPreviewQuery.data?.repair_incomplete
                  }
                  onClick={() => splitMutation.mutate('repair_incomplete')}
                >
                  {t('settings.trafficTracer.sessions.repairSplit', {
                    count: splitPreviewQuery.data?.repair_incomplete ?? 0,
                  })}
                </Button>
              </Stack>
            )}
          </Paper>
        )}

        {splitJob && (
          <Paper variant="outlined" sx={{ p: 1.5 }}>
            <Stack spacing={1}>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
                <Typography variant="body2" sx={{ flex: 1, fontWeight: 600 }}>
                  {t('settings.trafficTracer.sessions.splitJob')}:{' '}
                  {splitJob.message}
                </Typography>
                <Chip size="small" label={splitJob.state} />
                {splitJobActive && (
                  <Button
                    size="small"
                    color="warning"
                    disabled={splitCancelMutation.isPending}
                    onClick={() => splitCancelMutation.mutate()}
                  >
                    {t('settings.trafficTracer.common.actions.cancel')}
                  </Button>
                )}
              </Stack>
              <LinearProgress
                variant="determinate"
                value={Math.max(0, Math.min(100, splitJob.progress * 100))}
              />
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
                  if (scopeId) {
                    setSelectedSession({ scopeId, sessionId })
                    if (selection?.source === 'manual') {
                      persistManualWorkspace(
                        selection.scope.directory,
                        page,
                        sessionId,
                      )
                    }
                  }
                }}
              />
            ))}
          </Stack>
        )}

        {selection && totalPages > 1 && (
          <Pagination
            page={Math.min(page, totalPages)}
            count={totalPages}
            onChange={(_, nextPage) => {
              setPageState({ scopeId, page: nextPage })
              if (selection?.source === 'manual') {
                persistManualWorkspace(
                  selection.scope.directory,
                  nextPage,
                  selectedSessionId,
                )
              }
            }}
            sx={{ alignSelf: 'center' }}
          />
        )}
      </Stack>
      <TrafficTracerSessionDetail
        sessionId={selectedSessionId}
        workspaceRoot={workspaceRoot}
        onClose={() => {
          setSelectedSession(null)
          if (selection?.source === 'manual') {
            persistManualWorkspace(selection.scope.directory, page, null)
          }
        }}
      />
    </Paper>
  )
}
