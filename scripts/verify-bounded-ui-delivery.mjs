// Read-only verification of an isolated mixed_notifications run.
import assert from 'node:assert/strict'
import { readFileSync, readdirSync } from 'node:fs'
import { resolve } from 'node:path'
const root = process.argv[2]
assert.ok(
  root,
  'Usage: node scripts/verify-bounded-ui-delivery.mjs REPORT_DIRECTORY',
)
const report = JSON.parse(readFileSync(resolve(root, 'result.json'), 'utf8'))
assert.equal(report.status, 'passed')
assert.equal(report.passed, true)
assert.equal(report.fault, 'mixed_notifications')
assert.equal(report.js_errors, 0)
assert.equal(report.page_metrics.cleanupFailures, 0)
const samples = readFileSync(resolve(root, 'samples.jsonl'), 'utf8')
  .trim()
  .split('\n')
  .map(JSON.parse)
for (const sample of samples) {
  const d = sample.diagnostics
  assert.ok(d.progress_delivery.pending <= 1)
  assert.ok(d.progress_delivery.in_flight <= 1)
  assert.ok(d.notification_delivery.pending <= 32)
  assert.equal(d.notification_delivery.journal_failures, 0)
  assert.equal(d.notification_delivery.dispatch_failures, 0)
}
assert.ok(
  samples.some(
    (s) => s.diagnostics.recovery_delivery.snapshot?.status === 'complete',
  ),
)
const directory = resolve(root, 'diagnostics/worker-notifications')
const records = readdirSync(directory)
  .filter((name) => name.endsWith('.jsonl'))
  .flatMap((name) =>
    readFileSync(resolve(directory, name), 'utf8')
      .trim()
      .split('\n')
      .filter(Boolean)
      .map(JSON.parse),
  )
assert.ok(records.length > 100)
assert.ok(records.some((r) => r.method === 'worker.log'))
assert.ok(records.some((r) => r.method === 'job.completed'))
records.forEach((record, index) => {
  assert.equal(record.params.job_id, `synthetic-${2 * (index + 1)}`)
})
const d = report.diagnostics
assert.ok(d.notification_delivery.sequence > 0)
assert.ok(d.notification_delivery.sequence <= report.seconds * 5 + 2)
assert.ok(d.progress_delivery.counters.submitted <= report.seconds * 5 + 2)
console.log(
  JSON.stringify({
    status: 'passed',
    journal_records: records.length,
    progress: d.progress_delivery,
    notifications: d.notification_delivery,
  }),
)
