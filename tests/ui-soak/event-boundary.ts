// Relative import bypasses the exact API alias; delivery still uses native Tauri.
import { listen as nativeListen } from '../../node_modules/@tauri-apps/api/event'
import type {
  EventCallback,
  EventName,
  Options,
  UnlistenFn,
} from '../../node_modules/@tauri-apps/api/event'

export const subscriptions = { active: 0, pending: 0, peak: 0 }
export async function listen<T>(
  event: EventName,
  handler: EventCallback<T>,
  options?: Options,
): Promise<UnlistenFn> {
  subscriptions.pending++
  let release: UnlistenFn
  try {
    release = await nativeListen(event, handler, options)
  } finally {
    subscriptions.pending--
  }
  subscriptions.active++
  subscriptions.peak = Math.max(subscriptions.peak, subscriptions.active)
  let disposed = false
  return () => {
    if (disposed) return
    disposed = true
    try {
      release()
    } finally {
      subscriptions.active--
    }
  }
}
