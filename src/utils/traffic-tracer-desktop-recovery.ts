import { recoverySnapshots } from './traffic-tracer-recovery-snapshots'

export const DESKTOP_RECOVERY_EVENT = 'traffictracer-desktop-recovered'

export function recoveryGenerationDispatcher(
  recover: (generation: number) => void,
) {
  let latest = 0
  return (generation: unknown) => {
    if (
      typeof generation !== 'number' ||
      !Number.isSafeInteger(generation) ||
      generation <= latest
    )
      return
    latest = generation
    recover(generation)
  }
}

// Shared by heartbeat fallback and native notifications, across page remounts.
export const announceDesktopRecovery = recoveryGenerationDispatcher(
  (generation) => {
    recoverySnapshots.begin(generation)
    window.dispatchEvent(new Event(DESKTOP_RECOVERY_EVENT))
  },
)

/** The backend generation survives delayed JS timers and missed visibility events. */
export function desktopRecoveryObserver(recover: () => void) {
  let previous: number | undefined
  return (value: unknown) => {
    if (!value || typeof value !== 'object') return
    const snapshot = value as Record<string, unknown>
    const generation = snapshot.unlock_generation
    if (
      typeof generation !== 'number' ||
      !Number.isSafeInteger(generation) ||
      generation < 0
    )
      return
    if (snapshot.session_lock === 'locked') {
      previous ??= generation
      return
    }
    if (snapshot.session_lock !== 'unlocked') return
    if (previous !== undefined && generation > previous) recover()
    previous = generation
  }
}
