import { DESKTOP_RECOVERY_EVENT } from './traffic-tracer-desktop-recovery'
import { recoverySnapshots } from './traffic-tracer-recovery-snapshots'

/** Poll authoritative snapshots, not a backlog of missed timer ticks.
 * Callers own error reporting and disposal/revision guards for late responses.
 * Visibility is a scheduling hint, never proof that the OS session is locked.
 */
export function startVisibleSnapshotPoll(
  refresh: () => Promise<boolean | void>,
  intervalMs: number,
) {
  let disposed = false
  let inFlight = false
  let recoveryPending = false
  const reader = recoverySnapshots.register()
  const visible = () => document.visibilityState === 'visible'
  const poll = async () => {
    if (disposed || !visible() || inFlight) return
    inFlight = true
    const generation = recoverySnapshots.current()
    try {
      const refreshed = await refresh()
      if (!disposed) reader.finish(generation, refreshed === true)
    } catch {
      if (!disposed) reader.finish(generation, false)
    } finally {
      inFlight = false
      if (recoveryPending) {
        recoveryPending = false
        if (!disposed && visible()) void poll()
      }
    }
  }
  const recover = () => {
    if (disposed || !visible()) return
    if (inFlight) recoveryPending = true
    else void poll()
  }
  const visibilityChanged = () => {
    if (!visible()) recoveryPending = false
    else recover()
  }
  document.addEventListener('visibilitychange', visibilityChanged)
  window.addEventListener('pageshow', recover)
  window.addEventListener(DESKTOP_RECOVERY_EVENT, recover)
  const interval = window.setInterval(() => void poll(), intervalMs)
  void poll()
  return () => {
    disposed = true
    reader.dispose()
    recoveryPending = false
    window.clearInterval(interval)
    document.removeEventListener('visibilitychange', visibilityChanged)
    window.removeEventListener('pageshow', recover)
    window.removeEventListener(DESKTOP_RECOVERY_EVENT, recover)
  }
}
