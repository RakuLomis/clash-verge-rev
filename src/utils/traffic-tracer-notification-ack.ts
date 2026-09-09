import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import { createReceiptSender } from './traffic-tracer-receipt-sender'

export async function installNotificationAcknowledgement() {
  const receipts = createReceiptSender((sequence) =>
    invoke('tt_notification_ack', { sequence }),
  )
  let unlisten: () => void
  try {
    unlisten = await listen<number>(
      'traffictracer://notification-receipt',
      ({ payload }) => receipts.offer(payload),
    )
  } catch (error) {
    receipts.dispose()
    throw error
  }
  let disposed = false
  const dispose = () => {
    if (disposed) return
    disposed = true
    receipts.dispose()
    unlisten()
  }
  try {
    await invoke('tt_notification_ack', { sequence: 0 })
  } catch (error) {
    dispose()
    throw error
  }
  return dispose
}
