import { afterEach, expect, it, vi } from 'vitest'

const api = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
  announce: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: api.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: api.listen }))
vi.mock('./traffic-tracer-desktop-recovery', () => ({
  announceDesktopRecovery: api.announce,
}))
import { installNativeDesktopRecovery } from './traffic-tracer-native-recovery'
import {
  RECOVERY_CONFIRMATION_RETRY_EVENT,
  RECOVERY_SNAPSHOT_EVENT,
} from './traffic-tracer-recovery-snapshots'

it('retains a failed snapshot confirmation without a retry loop and retries on heartbeat', async () => {
  api.listen.mockResolvedValue(vi.fn())
  api.invoke.mockResolvedValue(undefined)
  const dispose = await installNativeDesktopRecovery()
  api.invoke.mockRejectedValueOnce(new Error('temporary IPC failure'))
  const report = { generation: 1, status: 'complete', participants: 2 }
  window.dispatchEvent(
    new CustomEvent(RECOVERY_SNAPSHOT_EVENT, { detail: report }),
  )
  await Promise.resolve()
  await Promise.resolve()
  const calls = api.invoke.mock.calls.length
  await Promise.resolve()
  expect(api.invoke).toHaveBeenCalledTimes(calls)
  window.dispatchEvent(new Event(RECOVERY_CONFIRMATION_RETRY_EVENT))
  await Promise.resolve()
  await Promise.resolve()
  expect(api.invoke).toHaveBeenLastCalledWith('tt_desktop_recovery_snapshot', {
    report,
  })
  expect(api.invoke).toHaveBeenCalledTimes(calls + 1)
  window.dispatchEvent(new Event(RECOVERY_CONFIRMATION_RETRY_EVENT))
  expect(api.invoke).toHaveBeenCalledTimes(calls + 1)
  dispose()
})

afterEach(() => vi.resetAllMocks())

it('registers before enabling, announces without a heartbeat, and cleans up', async () => {
  const release = vi.fn()
  api.listen.mockResolvedValue(release)
  api.invoke.mockResolvedValue(undefined)
  const dispose = await installNativeDesktopRecovery()
  expect(api.invoke).toHaveBeenCalledWith('tt_desktop_recovery_ack', {
    generation: 0,
  })
  const callback = api.listen.mock.calls[0][1]
  callback({ payload: 2 })
  expect(api.announce).toHaveBeenCalledWith(2)
  expect(api.invoke).toHaveBeenLastCalledWith('tt_desktop_recovery_ack', {
    generation: 2,
  })
  dispose()
  dispose()
  callback({ payload: 3 })
  expect(api.announce).toHaveBeenCalledOnce()
  expect(release).toHaveBeenCalledOnce()
})

it('cleans registration when readiness IPC fails', async () => {
  const release = vi.fn()
  api.listen.mockResolvedValue(release)
  api.invoke.mockRejectedValue(new Error('unavailable'))
  await expect(installNativeDesktopRecovery()).rejects.toThrow('unavailable')
  expect(release).toHaveBeenCalledOnce()
})
