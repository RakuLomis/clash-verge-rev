/** One retained receipt and one request. Retry failed IPC, never emitted data. */
export function createReceiptSender(
  send: (sequence: number) => Promise<unknown>,
) {
  let latest: number | undefined
  let pending = false
  let disposed = false
  const flush = async () => {
    if (disposed || pending || latest === undefined) return
    const sequence = latest
    latest = undefined
    pending = true
    try {
      await send(sequence)
    } catch {
      if (!disposed) latest = Math.max(latest ?? 0, sequence)
    } finally {
      pending = false
      if (latest !== undefined && latest !== sequence && !disposed) void flush()
    }
  }
  const timer = window.setInterval(() => {
    void flush()
  }, 2000)
  return {
    offer(sequence: number) {
      if (disposed || !Number.isSafeInteger(sequence) || sequence <= 0) return
      latest = Math.max(latest ?? 0, sequence)
      void flush()
    },
    dispose() {
      disposed = true
      latest = undefined
      window.clearInterval(timer)
    },
  }
}
