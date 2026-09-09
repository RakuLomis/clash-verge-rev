import { expect, it, vi } from 'vitest'

import { createRecoverySnapshotTracker } from './traffic-tracer-recovery-snapshots'

it('requires all mounted readers and ignores pre-recovery responses', () => {
  const notify = vi.fn()
  const tracker = createRecoverySnapshotTracker(notify)
  const a = tracker.register(),
    b = tracker.register()
  tracker.begin(1)
  a.finish(0, true)
  b.finish(1, true)
  expect(notify).toHaveBeenLastCalledWith({
    generation: 1,
    status: 'pending',
    participants: 2,
  })
  a.finish(1, false)
  expect(notify).toHaveBeenLastCalledWith({
    generation: 1,
    status: 'failed',
    participants: 2,
  })
  a.finish(1, true)
  expect(notify).toHaveBeenLastCalledWith({
    generation: 1,
    status: 'complete',
    participants: 2,
  })
  tracker.begin(2)
  b.finish(1, true)
  expect(notify).toHaveBeenLastCalledWith({
    generation: 2,
    status: 'pending',
    participants: 2,
  })
})

it('distinguishes no displayed readers from successful reads and ignores disposed responses', () => {
  const notify = vi.fn()
  const tracker = createRecoverySnapshotTracker(notify)
  tracker.begin(1)
  expect(notify).toHaveBeenLastCalledWith({
    generation: 1,
    status: 'not_displayed',
    participants: 0,
  })
  const a = tracker.register()
  expect(notify).toHaveBeenLastCalledWith({
    generation: 1,
    status: 'pending',
    participants: 1,
  })
  a.dispose()
  a.finish(1, true)
  expect(notify).toHaveBeenLastCalledWith({
    generation: 1,
    status: 'not_displayed',
    participants: 0,
  })
})
