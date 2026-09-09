import { expect, it, vi } from 'vitest'

import { ownTrafficTracerSubscriptions } from './traffic-tracer-subscriptions'

it('cleans each registration and remains idempotent', async () => {
  const release = vi.fn()
  const dispose = ownTrafficTracerSubscriptions(
    [Promise.resolve(release), Promise.resolve(release)],
    vi.fn(),
  )
  await Promise.resolve()
  dispose()
  dispose()
  expect(release).toHaveBeenCalledTimes(2)
})

it('a cleanup exception does not prevent releasing other subscriptions', async () => {
  const failed = vi.fn(() => {
    throw new Error('injected release failure')
  })
  const success = vi.fn()
  const error = vi.fn()
  const dispose = ownTrafficTracerSubscriptions(
    [Promise.resolve(failed), Promise.resolve(success)],
    error,
  )
  await Promise.resolve()
  dispose()
  expect(success).toHaveBeenCalledOnce()
  expect(error).toHaveBeenCalledOnce()
})

it('releases late successes even after a sibling rejection and disposal', async () => {
  let resolve!: (release: () => void) => void
  const late = new Promise<() => void>((yes) => {
    resolve = yes
  })
  const error = vi.fn()
  const dispose = ownTrafficTracerSubscriptions(
    [Promise.reject(new Error('registration failed')), late],
    error,
  )
  dispose()
  const release = vi.fn()
  resolve(release)
  await Promise.resolve()
  await Promise.resolve()
  expect(release).toHaveBeenCalledOnce()
  expect(error).toHaveBeenCalledOnce()
})
