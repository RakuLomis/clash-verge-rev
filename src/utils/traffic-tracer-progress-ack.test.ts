import { expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: api.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: api.listen }))
import { installProgressAcknowledgement } from './traffic-tracer-progress-ack'

it('registers before enabling delivery and acknowledges only valid sequences', async () => {
  api.invoke.mockResolvedValue(undefined)
  const dispose = vi.fn()
  api.listen.mockResolvedValue(dispose)
  const stop = await installProgressAcknowledgement()
  expect(api.invoke).toHaveBeenCalledWith('tt_progress_ack', { sequence: 0 })
  const callback = api.listen.mock.calls.at(-1)![1]
  callback({ payload: { delivery_sequence: 7 } })
  expect(api.invoke).toHaveBeenLastCalledWith('tt_progress_ack', {
    sequence: 7,
  })
  const count = api.invoke.mock.calls.length
  callback({ payload: { delivery_sequence: -1 } })
  callback({ payload: {} })
  expect(api.invoke).toHaveBeenCalledTimes(count)
  stop()
  expect(dispose).toHaveBeenCalledOnce()
})

it('releases registration if enabling delivery fails', async () => {
  const dispose = vi.fn()
  api.listen.mockResolvedValue(dispose)
  api.invoke.mockRejectedValueOnce(new Error('IPC failed'))
  await expect(installProgressAcknowledgement()).rejects.toThrow('IPC failed')
  expect(dispose).toHaveBeenCalledOnce()
})
