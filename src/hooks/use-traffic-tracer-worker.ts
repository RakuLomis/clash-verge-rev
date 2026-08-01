import { useQuery, useQueryClient } from '@tanstack/react-query'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useEffect } from 'react'

import {
  getTrafficTracerCaptureLock,
  getTrafficTracerEnvironment,
} from '@/services/cmds'
import type {
  EnvironmentRequest,
  WorkerLogEvent,
  WorkerReadyEvent,
} from '@/types/traffic-tracer'

export const trafficTracerCaptureLockKey = [
  'trafficTracer',
  'captureLock',
] as const

export const trafficTracerEnvironmentKey = (request: EnvironmentRequest) =>
  ['trafficTracer', 'environment', request] as const

export function useTrafficTracerWorker(
  request: EnvironmentRequest | null,
  enabled = true,
) {
  const queryClient = useQueryClient()
  const environmentQuery = useQuery({
    queryKey: request
      ? trafficTracerEnvironmentKey(request)
      : ['trafficTracer', 'environment', 'disabled'],
    queryFn: () => getTrafficTracerEnvironment(request!),
    enabled: enabled && request !== null,
    retry: 1,
  })
  const captureLockQuery = useQuery({
    queryKey: trafficTracerCaptureLockKey,
    queryFn: getTrafficTracerCaptureLock,
    enabled,
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
        if (payload.code?.startsWith('RECOVERY_')) invalidate()
      }),
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
          '[TrafficTracer] Worker event registration failed:',
          error,
        ),
      )

    return () => {
      disposed = true
      unlisteners.forEach((unlisten) => unlisten())
      unlisteners = []
    }
  }, [enabled, queryClient])

  return {
    environment: environmentQuery.data,
    environmentQuery,
    captureLock: captureLockQuery.data,
    captureLockQuery,
  }
}
