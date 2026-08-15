import { describe, expect, it, vi } from 'vitest'

vi.mock('tauri-plugin-mihomo-api', () => ({
  delayProxyByName: vi.fn(),
}))

import { DelayManager, type DelayUpdate } from './delay'

const flushNotifications = async () => {
  await new Promise<void>((resolve) =>
    window.requestAnimationFrame(() => resolve()),
  )
}

describe('DelayManager subscriptions', () => {
  it('notifies every subscriber for the same proxy', async () => {
    const manager = new DelayManager()
    const first = vi.fn<(update: DelayUpdate) => void>()
    const second = vi.fn<(update: DelayUpdate) => void>()

    manager.setListener('proxy-a', 'group-a', first)
    manager.setListener('proxy-a', 'group-a', second)
    manager.setDelay('proxy-a', 'group-a', -2)
    manager.setDelay('proxy-a', 'group-a', 123)
    await flushNotifications()

    expect(first.mock.calls.map(([update]) => update.delay)).toEqual([-2, 123])
    expect(second.mock.calls.map(([update]) => update.delay)).toEqual([-2, 123])
  })

  it('unsubscribes only the listener that owns the cleanup', async () => {
    const manager = new DelayManager()
    const first = vi.fn<(update: DelayUpdate) => void>()
    const second = vi.fn<(update: DelayUpdate) => void>()
    const unsubscribeFirst = manager.setListener('proxy-a', 'group-a', first)
    manager.setListener('proxy-a', 'group-a', second)

    unsubscribeFirst()
    manager.setDelay('proxy-a', 'group-a', 88)
    await flushNotifications()

    expect(first).not.toHaveBeenCalled()
    expect(second).toHaveBeenCalledOnce()
  })

  it('notifies all group subscribers when a proxy delay changes', async () => {
    const manager = new DelayManager()
    const first = vi.fn()
    const second = vi.fn()

    manager.setGroupListener('group-a', first)
    const unsubscribeSecond = manager.setGroupListener('group-a', second)
    manager.setDelay('proxy-a', 'group-a', 45)
    await flushNotifications()

    expect(first).toHaveBeenCalledOnce()
    expect(second).toHaveBeenCalledOnce()

    unsubscribeSecond()
    manager.setDelay('proxy-a', 'group-a', 46)
    await flushNotifications()

    expect(first).toHaveBeenCalledTimes(2)
    expect(second).toHaveBeenCalledOnce()
  })
})
