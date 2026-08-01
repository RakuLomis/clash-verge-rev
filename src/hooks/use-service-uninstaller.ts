import { useCallback } from 'react'

import { useTrafficTracerCaptureLock } from '@/hooks/use-traffic-tracer-worker'
import { restartCore, stopCore, uninstallService } from '@/services/cmds'
import { showNotice } from '@/services/notice-service'

import { useSystemState } from './use-system-state'

const executeWithErrorHandling = async (
  operation: () => Promise<void>,
  loadingKey: string,
  successKey?: string,
) => {
  try {
    showNotice.info(loadingKey)
    await operation()
    if (successKey) {
      showNotice.success(successKey)
    }
  } catch (err) {
    showNotice.error(err)
    throw err
  }
}

export const useServiceUninstaller = () => {
  const { captureLock, captureLockReason } = useTrafficTracerCaptureLock()
  const { mutateSystemState } = useSystemState()

  const uninstallServiceAndRestartCore = useCallback(async () => {
    if (captureLock?.locked) {
      throw new Error(captureLockReason ?? 'TrafficTracer capture is active')
    }
    try {
      await executeWithErrorHandling(
        () => stopCore(),
        'settings.statuses.clash.stopping',
      )
      await executeWithErrorHandling(
        () => uninstallService(),
        'settings.statuses.clashService.uninstalling',
        'settings.feedback.notifications.clashService.uninstallSuccess',
      )
    } catch (ignore) {
    } finally {
      await executeWithErrorHandling(
        () => restartCore(),
        'settings.statuses.clash.restarting',
        'settings.feedback.notifications.clash.restartSuccess',
      )
      await mutateSystemState()
    }
  }, [captureLock?.locked, captureLockReason, mutateSystemState])

  return { uninstallServiceAndRestartCore }
}
