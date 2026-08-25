import { useQuery, useQueryClient } from '@tanstack/react-query'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
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
  WorkerReadyEvent,
} from '@/types/traffic-tracer'

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
) {
  const queryClient = useQueryClient()
  const [workerActivity, setWorkerActivity] =
    useState<WorkerStartupActivity | null>(restoredWorkerActivity)
  const captureLockState = useTrafficTracerCaptureLock(enabled)
  const environmentQuery = useQuery({
    queryKey: request
      ? trafficTracerEnvironmentKey(request)
      : ['trafficTracer', 'environment', 'disabled'],
    queryFn: () => {
      if (!request)
        throw new Error('TrafficTracer environment request is missing')
      return getTrafficTracerEnvironment(request)
    },
    enabled: enabled && request !== null,
    retry: 1,
  })

  useEffect(() => {
    if (!enabled) return

    let disposed = false
    let unlisteners: UnlistenFn[] = []
    const invalidate = () => {
      void queryClient.invalidateQueries({
        queryKey: ['trafficTracer', 'environment'],
      })
    }

    Promise.all([
      listen<WorkerReadyEvent>('traffictracer://worker-ready', invalidate),
      listen<WorkerLogEvent>('traffictracer://worker-log', ({ payload }) => {
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
        if (payload.code?.startsWith('RECOVERY_')) invalidate()
      }),
    ])
      .then((registered) => {
        if (disposed) {
          registered.forEach((unlisten) => {
            unlisten()
          })
        } else {
          unlisteners = registered
        }
      })
      .catch((error) =>
        console.error(
          '[TrafficTracer] Worker event registration failed:',
          error,
        ),
      )

    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => {
        unlisten()
      })
      unlisteners = []
    }
  }, [enabled, queryClient])

  return {
    environment: environmentQuery.data,
    environmentQuery,
    workerActivity,
    ...captureLockState,
  }
}
