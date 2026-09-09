import { useQuery, useQueryClient } from '@tanstack/react-query'
import { listen } from '@tauri-apps/api/event'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'

import {
  getTrafficTracerCaptureLock,
  getTrafficTracerEnvironment,
} from '@/services/cmds'
import type {
  CaptureLockSnapshot,
  EnvironmentRequest,
  WorkerLogEvent,
} from '@/types/traffic-tracer'
import { ownTrafficTracerSubscriptions } from '@/utils/traffic-tracer-subscriptions'

export const trafficTracerCaptureLockKey = [
  'trafficTracer',
  'captureLock',
] as const
const WORKER_ACTIVITY_STORAGE_KEY = 'traffictracer.workerActivity.v1'

export interface WorkerStartupActivity {
  at: string
  code: string
  message: string
  timing: NonNullable<WorkerLogEvent['timing']>
}

function restoredWorkerActivity(): WorkerStartupActivity | null {
  try {
    const stored = localStorage.getItem(WORKER_ACTIVITY_STORAGE_KEY)
    if (!stored) return null
    const activity = JSON.parse(stored) as WorkerStartupActivity
    if (
      typeof activity.at !== 'string' ||
      typeof activity.code !== 'string' ||
      typeof activity.message !== 'string' ||
      typeof activity.timing?.operation !== 'string' ||
      typeof activity.timing?.duration_ms !== 'number'
    ) {
      throw new Error('invalid Worker activity')
    }
    return activity
  } catch {
    localStorage.removeItem(WORKER_ACTIVITY_STORAGE_KEY)
    return null
  }
}

export const trafficTracerEnvironmentKey = (request: EnvironmentRequest) =>
  ['trafficTracer', 'environment', request] as const

export function formatTrafficTracerCaptureLock(
  captureLock: CaptureLockSnapshot | undefined,
  fallback = 'TrafficTracer capture is active.',
  formatJob = (id: string) => `Job ${id}`,
) {
  if (!captureLock?.locked) return null
  const reason =
    !captureLock.reason ||
    captureLock.reason === 'TrafficTracer capture is active'
      ? fallback
      : captureLock.reason
  return captureLock.job_id
    ? `${reason} (${formatJob(captureLock.job_id)})`
    : reason
}

export function useTrafficTracerCaptureLock(enabled = true) {
  const { t } = useTranslation()
  const captureLockQuery = useQuery({
    queryKey: trafficTracerCaptureLockKey,
    queryFn: getTrafficTracerCaptureLock,
    enabled,
    refetchInterval: ({ state }) => (state.data?.locked ? 1000 : 5000),
  })

  return {
    captureLock: captureLockQuery.data,
    captureLockQuery,
    captureLockReason: formatTrafficTracerCaptureLock(
      captureLockQuery.data,
      t('settings.trafficTracer.locks.captureActive'),
      (id) => t('settings.trafficTracer.locks.job', { id }),
    ),
  }
}

export function useTrafficTracerWorker(
  request: EnvironmentRequest | null,
  enabled = true,
  diagnosticsPaused = false,
) {
  const queryClient = useQueryClient()
  const [workerActivity, setWorkerActivity] =
    useState<WorkerStartupActivity | null>(restoredWorkerActivity)
  const captureLockState = useTrafficTracerCaptureLock(enabled)
  const workspaceLocked = captureLockState.captureLock?.locked === true
  const environmentQuery = useQuery({
    queryKey: request
      ? trafficTracerEnvironmentKey(request)
      : ['trafficTracer', 'environment', 'disabled'],
    queryFn: () => {
      if (!request)
        throw new Error('TrafficTracer environment request is missing')
      return getTrafficTracerEnvironment(request)
    },
    // This command can switch/start a Worker. It is not a passive status read.
    enabled: false,
    retry: false,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    refetchOnMount: false,
  })

  const checkEnvironment = async (nextRequest: EnvironmentRequest) => {
    if (!enabled || diagnosticsPaused || workspaceLocked)
      throw new Error(
        'Environment checks are unavailable while capture is active.',
      )
    return queryClient.fetchQuery({
      queryKey: trafficTracerEnvironmentKey(nextRequest),
      queryFn: () => getTrafficTracerEnvironment(nextRequest),
      staleTime: 0,
      retry: false,
    })
  }

  useEffect(() => {
    if (!enabled) return

    let disposed = false
    const disposeSubscriptions = ownTrafficTracerSubscriptions(
      [
        listen<WorkerLogEvent>('traffictracer://worker-log', ({ payload }) => {
          if (disposed) return
          if (payload.timing) {
            const activity = {
              at: new Date().toISOString(),
              code: payload.code ?? 'WORKER_ACTIVITY',
              message: payload.message,
              timing: payload.timing,
            }
            localStorage.setItem(
              WORKER_ACTIVITY_STORAGE_KEY,
              JSON.stringify(activity),
            )
            setWorkerActivity(activity)
          }
        }),
      ],
      (error) =>
        console.error(
          '[TrafficTracer] Worker event registration failed:',
          error,
        ),
    )

    return () => {
      disposed = true
      disposeSubscriptions()
    }
  }, [enabled, queryClient])

  return {
    environment: environmentQuery.data,
    environmentQuery,
    checkEnvironment,
    workerActivity,
    ...captureLockState,
  }
}
