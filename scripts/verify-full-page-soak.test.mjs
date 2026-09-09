import assert from 'node:assert/strict'
import { test } from 'node:test'

import { validateFullPageReport } from './verify-full-page-soak.mjs'

const valid = () => ({
  schema_version: 2,
  status: 'passed',
  passed: true,
  scope: 'native_full_page_mocked_backend',
  full_ui_soak: true,
  first_failure: null,
  js_errors: 0,
  wall_elapsed_ms: 60000,
  seconds: 60,
  received: 100,
  rendered_sequence: 100,
  remounts: 2,
  page_metrics: {
    mode: 'full',
    denied: 0,
    errorBoundary: false,
    detailDialogs: 1,
    details: 1,
    paginationClicks: 1,
    batchSelections: 1,
    pipelineSelections: 1,
  },
})
test('accepts complete full-page coverage', () =>
  validateFullPageReport(valid()))
test('rejects component-only results', () => {
  const report = valid()
  report.scope = 'native_bridge_react_progress'
  assert.throws(() => validateFullPageReport(report))
})
test('rejects missing interactions even when the native report says passed', () => {
  for (const key of [
    'detailDialogs',
    'details',
    'paginationClicks',
    'batchSelections',
    'pipelineSelections',
  ]) {
    const report = valid()
    report.page_metrics[key] = 0
    assert.throws(() => validateFullPageReport(report), key)
  }
})
test('rejects errors, denied commands and shortened duration', () => {
  for (const mutate of [
    (report) => {
      report.js_errors = 1
    },
    (report) => {
      report.page_metrics.denied = 1
    },
    (report) => {
      report.page_metrics.errorBoundary = true
    },
    (report) => {
      report.wall_elapsed_ms = 1000
    },
  ]) {
    const report = valid()
    mutate(report)
    assert.throws(() => validateFullPageReport(report))
  }
})
test('requires actual slow recovery and subscription cleanup coverage', () => {
  const report = valid()
  Object.assign(report.page_metrics, {
    scenario: 'slow_recovery',
    cleanupChecks: 2,
    cleanupFailures: 0,
    delayedCalls: 5,
    statusFailures: 1,
    resumeCalls: 1,
    visibleStatusErrors: 1,
    clearedStatusErrors: 1,
    largeIndexReads: 2,
    analysisRowsPeak: 100,
  })
  validateFullPageReport(report)
  for (const key of [
    'cleanupChecks',
    'delayedCalls',
    'statusFailures',
    'resumeCalls',
    'visibleStatusErrors',
    'clearedStatusErrors',
    'largeIndexReads',
    'analysisRowsPeak',
  ]) {
    const incomplete = structuredClone(report)
    incomplete.page_metrics[key] = 0
    assert.throws(() => validateFullPageReport(incomplete), key)
  }
  report.page_metrics.cleanupFailures = 1
  assert.throws(() => validateFullPageReport(report))
})
