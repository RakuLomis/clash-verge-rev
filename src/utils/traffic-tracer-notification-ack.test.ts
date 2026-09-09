import { afterEach, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: api.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: api.listen }))
import { installNotificationAcknowledgement } from './traffic-tracer-notification-ack'
afterEach(() => {
  vi.resetAllMocks()
  vi.useRealTimers()
})
it('acknowledges receipts and removes retry timer and listener on disposal', async () => {
  vi.useFakeTimers()
  const release = vi.fn()
  api.listen.mockResolvedValue(release)
  api.invoke.mockResolvedValue(undefined)
  const dispose = await installNotificationAcknowledgement()
  const callback = api.listen.mock.calls[0][1]
  callback({ payload: 3 })
  expect(api.invoke).toHaveBeenLastCalledWith('tt_notification_ack', {
    sequence: 3,
  })
  dispose()
  dispose()
  callback({ payload: 4 })
  expect(api.invoke).toHaveBeenLastCalledWith('tt_notification_ack', {
    sequence: 3,
  })
  expect(release).toHaveBeenCalledOnce()
  expect(vi.getTimerCount()).toBe(0)
})
it('cleans retry ownership when readiness fails', async () => {
  vi.useFakeTimers()
  const release = vi.fn()
  api.listen.mockResolvedValue(release)
  api.invoke.mockRejectedValue(new Error('unavailable'))
  await expect(installNotificationAcknowledgement()).rejects.toThrow(
    'unavailable',
  )
  expect(release).toHaveBeenCalledOnce()
  expect(vi.getTimerCount()).toBe(0)
})
