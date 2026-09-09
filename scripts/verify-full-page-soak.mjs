import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function validateFullPageReport(report) {
  assert.equal(report.schema_version, 2)
  assert.equal(report.status, 'passed')
  assert.equal(report.passed, true)
  assert.equal(report.scope, 'native_full_page_mocked_backend')
  assert.equal(report.full_ui_soak, true)
  assert.equal(report.first_failure, null)
  assert.equal(report.js_errors, 0)
  assert.ok(report.wall_elapsed_ms >= report.seconds * 1000)
  assert.ok(
    report.received > 0 && report.rendered_sequence > 0 && report.remounts > 0,
  )
  const page = report.page_metrics
  assert.equal(page.mode, 'full')
  assert.equal(page.denied, 0)
  assert.equal(page.errorBoundary, false)
  for (const key of [
    'detailDialogs',
    'details',
    'paginationClicks',
    'batchSelections',
    'pipelineSelections',
  ]) {
    assert.ok(page[key] > 0, `Missing full-page coverage: ${key}`)
  }
  if (page.scenario) {
    assert.ok(page.cleanupChecks > 0, 'Missing unmount cleanup checks')
    assert.equal(page.cleanupFailures, 0, 'Subscriptions survived page unmount')
  }
  if (page.scenario === 'slow_recovery') {
    for (const key of [
      'delayedCalls',
      'statusFailures',
      'resumeCalls',
      'visibleStatusErrors',
      'clearedStatusErrors',
      'largeIndexReads',
      'analysisRowsPeak',
    ]) {
      assert.ok(page[key] > 0, `Missing recovery coverage: ${key}`)
    }
    assert.ok(
      page.analysisRowsPeak <= 100,
      'Analysis rendered more than one page per index',
    )
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  assert.ok(
    process.argv[2],
    'Usage: node scripts/verify-full-page-soak.mjs REPORT_DIRECTORY',
  )
  const root = resolve(process.argv[2])
  const report = JSON.parse(
    await readFile(resolve(root, 'result.json'), 'utf8'),
  )
  validateFullPageReport(report)
  const samples = (await readFile(resolve(root, 'samples.jsonl'), 'utf8'))
    .trim()
    .split('\n')
    .map(JSON.parse)
  assert.equal(samples.length, report.samples)
  assert.ok(samples.every((sample) => !sample.failed && sample.js_errors === 0))
  const mature = samples.filter((sample) => sample.elapsed_seconds >= 30)
  const range = (values) => ({
    min: Math.min(...values),
    max: Math.max(...values),
  })
  console.log(
    JSON.stringify(
      {
        verified: true,
        root,
        report,
        resource_review_not_leak_verdict: mature.length
          ? {
              family_rss_kib: range(
                mature.map((sample) => sample.process_family.rss_sum_kib),
              ),
              queries: range(
                mature.map((sample) => sample.page_metrics?.queries ?? 0),
              ),
              native_fds: range(mature.map((sample) => sample.native_fd_count)),
              in_flight: range(
                mature.map((sample) => sample.page_metrics?.inFlight ?? 0),
              ),
              owned_subscriptions: range(
                mature.map(
                  (sample) => sample.page_metrics?.subscriptions?.active ?? 0,
                ),
              ),
              analysis_queries: range(
                mature.map(
                  (sample) => sample.page_metrics?.analysisQueries ?? 0,
                ),
              ),
            }
          : null,
      },
      null,
      2,
    ),
  )
}
