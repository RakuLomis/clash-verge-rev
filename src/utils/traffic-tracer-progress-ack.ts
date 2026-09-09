import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

import { createReceiptSender } from './traffic-tracer-receipt-sender'

/** One window-lifetime listener, independent of TrafficTracer page mounts.
 * ACK means JS received the event, not that React rendered or analysis finished.
 * The native gate additionally limits submission to five events per second.
 */
export async function installProgressAcknowledgement() {
  const receipts = createReceiptSender((sequence) =>
    invoke('tt_progress_ack', { sequence }),
  )
  let unlisten: () => void
  try {
    unlisten = await listen<{ delivery_sequence?: number }>(
      'traffictracer://job-progress',
      ({ payload }) => {
        const sequence = payload.delivery_sequence
        if (
          typeof sequence === 'number' &&
          Number.isSafeInteger(sequence) &&
          sequence > 0
        )
          receipts.offer(sequence)
      },
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
    await invoke('tt_progress_ack', { sequence: 0 })
  } catch (error) {
    dispose()
    throw error
  }
  return dispose
}
