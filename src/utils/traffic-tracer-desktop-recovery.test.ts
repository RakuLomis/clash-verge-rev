import { expect, it, vi } from 'vitest'

import {
  desktopRecoveryObserver,
  recoveryGenerationDispatcher,
} from './traffic-tracer-desktop-recovery'

it('deduplicates native and heartbeat generations, including invalid or stale input', () => {
  const refresh = vi.fn()
  const announce = recoveryGenerationDispatcher(refresh)
  for (const generation of [undefined, -1, 0, NaN, 1, 1, 0, 2, 2])
    announce(generation)
  expect(refresh).toHaveBeenCalledTimes(2)
})

it('also recovers when the first observation is locked', () => {
  const refresh = vi.fn()
  const observe = desktopRecoveryObserver(refresh)
  observe({ session_lock: 'locked', unlock_generation: 4 })
  observe({ session_lock: 'unlocked', unlock_generation: 5 })
  expect(refresh).toHaveBeenCalledOnce()
})

it('refreshes once per confirmed unlock even without visibilitychange', () => {
  const refresh = vi.fn()
  const observe = desktopRecoveryObserver(refresh)
  observe({ session_lock: 'unlocked', unlock_generation: 0 })
  observe({ session_lock: 'locked', unlock_generation: 0 })
  observe({ session_lock: 'unknown', unlock_generation: 1 })
  expect(refresh).not.toHaveBeenCalled()
  observe({ session_lock: 'unlocked', unlock_generation: 1 })
  observe({ session_lock: 'unlocked', unlock_generation: 1 })
  expect(refresh).toHaveBeenCalledOnce()
})

it('ignores unsupported responses and treats a new monitor as a baseline', () => {
  const refresh = vi.fn()
  const observe = desktopRecoveryObserver(refresh)
  for (const value of [
    null,
    undefined,
    {},
    { session_lock: 'unlocked', unlock_generation: -1 },
  ])
    observe(value)
  observe({ session_lock: 'unlocked', unlock_generation: 5 })
  expect(refresh).not.toHaveBeenCalled()
  observe({ session_lock: 'unlocked', unlock_generation: 6 })
  expect(refresh).toHaveBeenCalledOnce()
})
