export type RecoverySnapshotStatus =
  | 'pending'
  | 'complete'
  | 'failed'
  | 'not_displayed'
export type RecoverySnapshotReport = {
  generation: number
  status: RecoverySnapshotStatus
  participants: number
}
export const RECOVERY_SNAPSHOT_EVENT = 'traffictracer-recovery-snapshots'
export const RECOVERY_CONFIRMATION_RETRY_EVENT =
  'traffictracer-recovery-confirmation-retry'

/** Tracks the currently mounted pipeline/history readers, not all Worker state. */
export function createRecoverySnapshotTracker(
  notify: (report: RecoverySnapshotReport) => void,
) {
  let generation = 0
  let nextId = 0
  const readers = new Map<number, boolean | undefined>()
  let lastReport = ''
  const publish = () => {
    if (!generation) return
    const values = [...readers.values()]
    const status: RecoverySnapshotStatus =
      values.length === 0
        ? 'not_displayed'
        : values.some((value) => value === false)
          ? 'failed'
          : values.every((value) => value === true)
            ? 'complete'
            : 'pending'
    const report = { generation, status, participants: readers.size }
    const key = JSON.stringify(report)
    if (key !== lastReport) {
      lastReport = key
      notify(report)
    }
  }
  return {
    begin(next: number) {
      if (!Number.isSafeInteger(next) || next <= generation) return
      generation = next
      for (const id of readers.keys()) readers.set(id, undefined)
      publish()
    },
    current: () => generation,
    register() {
      const id = ++nextId
      readers.set(id, undefined)
      publish()
      return {
        finish(epoch: number, success: boolean) {
          if (epoch !== generation || !readers.has(id)) return
          readers.set(id, success)
          publish()
        },
        dispose() {
          readers.delete(id)
          publish()
        },
      }
    },
  }
}

export const recoverySnapshots = createRecoverySnapshotTracker((report) => {
  window.dispatchEvent(
    new CustomEvent(RECOVERY_SNAPSHOT_EVENT, { detail: report }),
  )
})
