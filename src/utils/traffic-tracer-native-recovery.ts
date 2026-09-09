import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import { announceDesktopRecovery } from './traffic-tracer-desktop-recovery'
import { createReceiptSender } from './traffic-tracer-receipt-sender'
import {
  RECOVERY_CONFIRMATION_RETRY_EVENT,
  RECOVERY_SNAPSHOT_EVENT,
  type RecoverySnapshotReport,
} from './traffic-tracer-recovery-snapshots'

/** Window-lifetime registration. Receipt ACK does not certify snapshot refresh. */
export async function installNativeDesktopRecovery() {
  let disposed = false
  const receipts = createReceiptSender((generation) =>
    invoke('tt_desktop_recovery_ack', { generation }),
  )
  let pending = false
  let latest: RecoverySnapshotReport | undefined
  const flush = async () => {
    if (pending || disposed || !latest) return
    const report = latest
    latest = undefined
    pending = true
    try {
      await invoke('tt_desktop_recovery_snapshot', { report })
    } catch {
      // Retain at most one report, retry on a later heartbeat/foreground event.
      // Never recursively retry the same failed request.
      if (!disposed) latest ??= report
    } finally {
      pending = false
      if (latest && latest !== report && !disposed) void flush()
    }
  }
  const snapshotChanged = (event: Event) => {
    latest = (event as CustomEvent<RecoverySnapshotReport>).detail
    void flush()
  }
  window.addEventListener(RECOVERY_SNAPSHOT_EVENT, snapshotChanged)
  const retry = () => {
    void flush()
  }
  window.addEventListener(RECOVERY_CONFIRMATION_RETRY_EVENT, retry)
  window.addEventListener('pageshow', retry)
  let unlisten: () => void
  try {
    unlisten = await listen<number>(
      'traffictracer://desktop-recovery',
      ({ payload }) => {
        if (disposed || !Number.isSafeInteger(payload) || payload <= 0) return
        announceDesktopRecovery(payload)
        receipts.offer(payload)
      },
    )
  } catch (error) {
    receipts.dispose()
    window.removeEventListener(RECOVERY_SNAPSHOT_EVENT, snapshotChanged)
    window.removeEventListener(RECOVERY_CONFIRMATION_RETRY_EVENT, retry)
    window.removeEventListener('pageshow', retry)
    throw error
  }
  const dispose = () => {
    if (disposed) return
    disposed = true
    receipts.dispose()
    latest = undefined
    window.removeEventListener(RECOVERY_SNAPSHOT_EVENT, snapshotChanged)
    window.removeEventListener(RECOVERY_CONFIRMATION_RETRY_EVENT, retry)
    window.removeEventListener('pageshow', retry)
    unlisten()
  }
  try {
    await invoke('tt_desktop_recovery_ack', { generation: 0 })
  } catch (error) {
    dispose()
    throw error
  }
  return dispose
}
