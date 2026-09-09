import { afterEach, expect, it, vi } from 'vitest'

import { DESKTOP_RECOVERY_EVENT } from './traffic-tracer-desktop-recovery'
import { startVisibleSnapshotPoll } from './traffic-tracer-visible-poll'

let stop: (() => void) | undefined
afterEach(() => {
  stop?.()
  vi.restoreAllMocks()
  vi.useRealTimers()
})

function visibility() {
  let state = 'visible'
  vi.spyOn(document, 'visibilityState', 'get').mockImplementation(
    () => state as DocumentVisibilityState,
  )
  return (next: string) => {
    state = next
    document.dispatchEvent(new Event('visibilitychange'))
  }
}

it('pauses hidden reads and refreshes immediately on return without replay', async () => {
  vi.useFakeTimers()
  const setVisible = visibility()
  const refresh = vi.fn(async () => {})
  stop = startVisibleSnapshotPoll(refresh, 1000)
  await vi.advanceTimersByTimeAsync(0)
  setVisible('hidden')
  await vi.advanceTimersByTimeAsync(60_000)
  expect(refresh).toHaveBeenCalledTimes(1)
  setVisible('visible')
  await vi.advanceTimersByTimeAsync(0)
  expect(refresh).toHaveBeenCalledTimes(2)
})

it('coalesces recovery during a slow read and never overlaps requests', async () => {
  vi.useFakeTimers()
  const setVisible = visibility()
  let resolve!: () => void
  const refresh = vi.fn(
    () =>
      new Promise<void>((done) => {
        resolve = done
      }),
  )
  stop = startVisibleSnapshotPoll(refresh, 1000)
  setVisible('hidden')
  setVisible('visible')
  window.dispatchEvent(new Event('pageshow'))
  window.dispatchEvent(new Event(DESKTOP_RECOVERY_EVENT))
  await vi.advanceTimersByTimeAsync(5000)
  expect(refresh).toHaveBeenCalledTimes(1)
  resolve()
  await vi.advanceTimersByTimeAsync(0)
  expect(refresh).toHaveBeenCalledTimes(2)
  stop()
  resolve()
  window.dispatchEvent(new Event('pageshow'))
  await vi.advanceTimersByTimeAsync(5000)
  expect(refresh).toHaveBeenCalledTimes(2)
  expect(vi.getTimerCount()).toBe(0)
})

it('does not start hidden and still polls when no visibility event accompanies reconnect', async () => {
  vi.useFakeTimers()
  const setVisible = visibility()
  setVisible('hidden')
  const refresh = vi.fn(async () => {})
  stop = startVisibleSnapshotPoll(refresh, 1000)
  expect(refresh).not.toHaveBeenCalled()
  setVisible('visible')
  await vi.advanceTimersByTimeAsync(3000)
  expect(refresh).toHaveBeenCalledTimes(4)
})
