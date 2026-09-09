import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function validateStallDiagnostics(report, scenario) {
  assert.ok(['driver_stall', 'js_stall'].includes(scenario))
  assert.equal(report.status, 'failed')
  assert.equal(report.passed, false)
  assert.equal(report.page_metrics.scenario, scenario)
  assert.equal(report.first_failure.reason, 'HEARTBEAT_STALLED')
  assert.ok(
    report.wall_elapsed_ms < report.seconds * 1000,
    'Failure was not finalized early',
  )
  const diagnostics = report.diagnostics
  assert.ok(
    diagnostics.native_main_loop_gap_ms < 3000,
    'Native main loop did not respond',
  )
  if (scenario === 'driver_stall') {
    assert.ok(
      diagnostics.event_probe_gap_ms < 2000,
      'Independent JS events also stalled',
    )
    assert.equal(diagnostics.event_probe.driverStage, 'injected_driver_stall')
    assert.ok(diagnostics.event_probe.driverAgeMs >= 8000)
  } else {
    // The independent probe and driver have different one-second phases.
    // This diagnostic comparison does not change Health's ten-second gate.
    assert.ok(
      diagnostics.event_probe_gap_ms >= 8000,
      'JS-main-thread stall not observed',
    )
  }
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  assert.ok(
    process.argv[2] && process.argv[3],
    'Usage: node scripts/verify-native-diagnostics.mjs REPORT_DIRECTORY driver_stall|js_stall',
  )
  const report = JSON.parse(
    await readFile(resolve(process.argv[2], 'result.json'), 'utf8'),
  )
  validateStallDiagnostics(report, process.argv[3])
  console.log(
    JSON.stringify({
      verified: true,
      scenario: process.argv[3],
      first_failure: report.first_failure,
      diagnostics: report.diagnostics,
    }),
  )
}
