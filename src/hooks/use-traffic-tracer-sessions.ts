import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useEffect } from 'react'

import { trafficTracerJobKey } from '@/hooks/use-capture-job'
import {
  getTrafficTracerSession,
  listTrafficTracerScopedSessions,
  listTrafficTracerSessions,
  startTrafficTracerAnalysis,
} from '@/services/cmds'
import type {
  AnalysisOptions,
  JobSnapshot,
  SessionListResult,
  SessionManifest,
} from '@/types/traffic-tracer'

export const trafficTracerSessionsKey = ['trafficTracer', 'sessions'] as const

export const trafficTracerScopedSessionsKey = (
  workspaceRoot: string,
  scopeId: string,
) => [...trafficTracerSessionsKey, 'scope', workspaceRoot, scopeId] as const
export const trafficTracerSessionDetailsKey = [
  'trafficTracer',
  'session',
] as const
export const trafficTracerSessionKey = (
  sessionId: string,
  workspaceRoot = '',
) => [...trafficTracerSessionDetailsKey, workspaceRoot, sessionId] as const
export const trafficTracerFlowsKey = ['trafficTracer', 'flows'] as const

export interface SessionPage {
  sessions: SessionManifest[]
  corrupt: SessionListResult['corrupt']
  offset: number
  limit: number
  total: number
}

export function paginateTrafficTracerSessions(
  result: SessionListResult | undefined,
  offset: number,
  limit: number,
): SessionPage {
  const safeOffset = Number.isFinite(offset)
    ? Math.max(0, Math.trunc(offset))
    : 0
  const safeLimit = Number.isFinite(limit) ? Math.max(1, Math.trunc(limit)) : 20
  const sessions = result?.sessions ?? []
  return {
    sessions: sessions.slice(safeOffset, safeOffset + safeLimit),
    corrupt: result?.corrupt ?? [],
    offset: safeOffset,
    limit: safeLimit,
    total: sessions.length,
  }
}

export function useTrafficTracerSessions(
  offset = 0,
  limit = 20,
  enabled = true,
  workspaceRoot = '',
) {
  const queryClient = useQueryClient()
  const sessionsQuery = useQuery({
    queryKey: [...trafficTracerSessionsKey, workspaceRoot],
    queryFn: listTrafficTracerSessions,
    enabled,
  })

  useEffect(() => {
    if (!enabled) return

    let disposed = false
    let unlisteners: UnlistenFn[] = []
    const refreshSessions = (_event: { payload: JobSnapshot }) => {
      void queryClient.invalidateQueries({ queryKey: trafficTracerSessionsKey })
      void queryClient.invalidateQueries({
        queryKey: trafficTracerSessionDetailsKey,
      })
      void queryClient.invalidateQueries({ queryKey: trafficTracerFlowsKey })
    }

    Promise.all([
      listen<JobSnapshot>('traffictracer://job-completed', refreshSessions),
      listen<JobSnapshot>('traffictracer://job-failed', refreshSessions),
      listen<JobSnapshot>('traffictracer://job-cancelled', refreshSessions),
    ])
      .then((registered) => {
        if (disposed) {
          registered.forEach((unlisten) => unlisten())
        } else {
          unlisteners = registered
        }
      })
      .catch((error) =>
        console.error(
          '[TrafficTracer] Session event registration failed:',
          error,
        ),
      )

    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => unlisten())
      unlisteners = []
    }
  }, [enabled, queryClient, workspaceRoot])

  return {
    ...paginateTrafficTracerSessions(sessionsQuery.data, offset, limit),
    sessionsQuery,
    refreshSessions: sessionsQuery.refetch,
  }
}

export function useTrafficTracerScopedSessions(
  scopeId: string | null,
  offset = 0,
  limit = 20,
  enabled = true,
  workspaceRoot = '',
) {
  const queryClient = useQueryClient()
  const sessionsQuery = useQuery({
    queryKey: scopeId
      ? trafficTracerScopedSessionsKey(workspaceRoot, scopeId)
      : [...trafficTracerSessionsKey, 'scope', workspaceRoot, 'none'],
    queryFn: () => listTrafficTracerScopedSessions(scopeId!),
    enabled: enabled && scopeId !== null,
  })

  useEffect(() => {
    if (!enabled || !scopeId) return

    let disposed = false
    let unlisteners: UnlistenFn[] = []
    const refreshSessions = () => {
      void queryClient.invalidateQueries({
        queryKey: trafficTracerScopedSessionsKey(workspaceRoot, scopeId),
      })
      void queryClient.invalidateQueries({
        queryKey: trafficTracerSessionDetailsKey,
      })
      void queryClient.invalidateQueries({ queryKey: trafficTracerFlowsKey })
    }

    Promise.all([
      listen<JobSnapshot>('traffictracer://job-completed', refreshSessions),
      listen<JobSnapshot>('traffictracer://job-failed', refreshSessions),
      listen<JobSnapshot>('traffictracer://job-cancelled', refreshSessions),
    ])
      .then((registered) => {
        if (disposed) registered.forEach((unlisten) => unlisten())
        else unlisteners = registered
      })
      .catch((error) =>
        console.error(
          '[TrafficTracer] Scoped Session event registration failed:',
          error,
        ),
      )

    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => unlisten())
      unlisteners = []
    }
  }, [enabled, queryClient, scopeId, workspaceRoot])

  return {
    ...paginateTrafficTracerSessions(sessionsQuery.data, offset, limit),
    scope: sessionsQuery.data?.scope,
    sessionsQuery,
    refreshSessions: sessionsQuery.refetch,
  }
}

export function useTrafficTracerSession(
  sessionId: string | null,
  enabled = true,
  workspaceRoot = '',
) {
  const queryClient = useQueryClient()
  const sessionQuery = useQuery({
    queryKey: sessionId
      ? trafficTracerSessionKey(sessionId, workspaceRoot)
      : ['trafficTracer', 'session', 'none'],
    queryFn: () => getTrafficTracerSession(sessionId!),
    enabled: enabled && sessionId !== null,
  })

  const analysisMutation = useMutation({
    mutationFn: (options?: Partial<AnalysisOptions>) => {
      if (!sessionId) throw new Error('No TrafficTracer Session is selected')
      return startTrafficTracerAnalysis(sessionId, options)
    },
    onSuccess: (snapshot) => {
      queryClient.setQueryData(trafficTracerJobKey(snapshot.job_id), snapshot)
      void queryClient.invalidateQueries({ queryKey: trafficTracerSessionsKey })
      if (sessionId) {
        void queryClient.invalidateQueries({
          queryKey: trafficTracerSessionKey(sessionId, workspaceRoot),
        })
      }
    },
  })

  return {
    session: sessionQuery.data,
    sessionQuery,
    startAnalysis: analysisMutation.mutateAsync,
    analysisMutation,
  }
}
