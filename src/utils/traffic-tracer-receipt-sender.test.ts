import { afterEach, expect, it, vi } from 'vitest'

import { createReceiptSender } from './traffic-tracer-receipt-sender'
afterEach(() => vi.useRealTimers())
it('retries only the failed receipt on a bounded timer and stops on disposal', async () => {
  vi.useFakeTimers()
  const send = vi
    .fn()
    .mockRejectedValueOnce(new Error('IPC'))
    .mockResolvedValue(undefined)
  const sender = createReceiptSender(send)
  sender.offer(1)
  await vi.advanceTimersByTimeAsync(1999)
  expect(send).toHaveBeenCalledTimes(1)
  await vi.advanceTimersByTimeAsync(1)
  expect(send).toHaveBeenCalledTimes(2)
  await vi.advanceTimersByTimeAsync(10000)
  expect(send).toHaveBeenCalledTimes(2)
  sender.dispose()
  sender.offer(2)
  expect(vi.getTimerCount()).toBe(0)
  expect(send).toHaveBeenCalledTimes(2)
})
it('coalesces receipts behind a slow IPC without concurrent calls', async () => {
  vi.useFakeTimers()
  let done!: () => void
  const send = vi.fn(
    () =>
      new Promise<void>((resolve) => {
        done = resolve
      }),
  )
  const sender = createReceiptSender(send)
  sender.offer(1)
  for (let n = 2; n <= 10000; n++) sender.offer(n)
  await vi.advanceTimersByTimeAsync(6000)
  expect(send).toHaveBeenCalledTimes(1)
  done()
  await vi.advanceTimersByTimeAsync(0)
  expect(send).toHaveBeenLastCalledWith(10000)
  expect(send).toHaveBeenCalledTimes(2)
  sender.dispose()
  done()
})
