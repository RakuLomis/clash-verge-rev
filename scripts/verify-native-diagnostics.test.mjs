import assert from 'node:assert/strict'
import { test } from 'node:test'

import { validateStallDiagnostics } from './verify-native-diagnostics.mjs'

const fixture = (scenario) => ({
  status: 'failed',
  passed: false,
  page_metrics: { scenario },
  seconds: 50,
  wall_elapsed_ms: 31000,
  first_failure: { reason: 'HEARTBEAT_STALLED' },
  diagnostics: {
    native_main_loop_gap_ms: 1000,
    event_probe_gap_ms: scenario === 'driver_stall' ? 500 : 11000,
    event_probe: { driverStage: 'injected_driver_stall', driverAgeMs: 9000 },
  },
})
test('distinguishes a stopped driver from a blocked JS event loop', () => {
  for (const scenario of ['driver_stall', 'js_stall'])
    validateStallDiagnostics(fixture(scenario), scenario)
  const driver = fixture('driver_stall')
  driver.diagnostics.event_probe_gap_ms = 11000
  assert.throws(() => validateStallDiagnostics(driver, 'driver_stall'))
  const js = fixture('js_stall')
  js.diagnostics.event_probe_gap_ms = 500
  assert.throws(() => validateStallDiagnostics(js, 'js_stall'))
  const offset = fixture('js_stall')
  offset.diagnostics.event_probe_gap_ms = 9807
  validateStallDiagnostics(offset, 'js_stall')
})
test('cannot turn recovery or a native stall into a passed negative control', () => {
  const report = fixture('driver_stall')
  report.status = 'passed'
  report.passed = true
  assert.throws(() => validateStallDiagnostics(report, 'driver_stall'))
  const native = fixture('js_stall')
  native.diagnostics.native_main_loop_gap_ms = 12000
  assert.throws(() => validateStallDiagnostics(native, 'js_stall'))
})
