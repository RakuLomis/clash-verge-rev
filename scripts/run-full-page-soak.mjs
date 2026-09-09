// Prebuild the isolated full-page example; never launches production Clash Verge.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'

import { validateFullPageReport } from './verify-full-page-soak.mjs'

const seconds = Number(process.argv[2] ?? 300)
assert.ok(
  Number.isInteger(seconds) && seconds >= 100 && seconds <= 14400,
  'Usage: node scripts/run-full-page-soak.mjs SECONDS (100..14400)',
)
const child = spawn(
  resolve('target/debug/examples/traffictracer-ui-soak'),
  [String(seconds)],
  {
    env: { ...process.env, TT_SOAK_FAULT: '', TT_SOAK_SAMPLE_DELAY_MS: '0' },
    stdio: ['ignore', 'pipe', 'pipe'],
  },
)
let directory
let output = ''
let timedOut = false
let failedExitTimer
const remember = (chunk) => {
  // Native reports are on disk; keep only bounded diagnostics in the runner.
  output = (output + chunk.toString()).slice(-16384)
  const found = output.match(/^SOAK_REPORT_DIR=([^\r\n]+)\r?\n/m)?.[1]
  if (!directory && found) {
    directory = found
    console.log(
      JSON.stringify({
        status: 'running',
        seconds,
        directory,
        isolated_pid: child.pid,
      }),
    )
  }
  if (!failedExitTimer && output.includes('SOAK_FINISHED passed=false')) {
    // The observer has persisted failure; do not wait hours if the main loop
    // cannot process its exit request. Never signal any production PID.
    failedExitTimer = setTimeout(() => child.kill('SIGKILL'), 15000)
  }
}
child.stdout.on('data', remember)
child.stderr.on('data', remember)
const timer = setTimeout(
  () => {
    timedOut = true
    child.kill('SIGKILL') // Only the isolated process created above.
  },
  (seconds + 45) * 1000,
)
try {
  const exit = await new Promise((resolveExit, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) => resolveExit({ code, signal }))
  })
  assert.equal(timedOut, false, 'Isolated window exceeded its deadline')
  assert.equal(exit.signal, null, 'Isolated window exited by signal')
  assert.equal(exit.code, 0, `Isolated window failed: ${output}`)
  assert.ok(directory, 'Missing report directory')
  const report = JSON.parse(
    await readFile(resolve(directory, 'result.json'), 'utf8'),
  )
  assert.equal(report.seconds, seconds)
  assert.equal(
    report.page_metrics?.scenario,
    'slow_recovery',
    'Rebuild with TT_SOAK_PAGE=full TT_SOAK_SCENARIO=slow_recovery',
  )
  validateFullPageReport(report)
  console.log(
    JSON.stringify({
      status: 'passed',
      seconds,
      directory,
      received: report.received,
      max_heartbeat_gap_ms: report.max_heartbeat_gap_ms,
      page_metrics: report.page_metrics,
    }),
  )
} finally {
  clearTimeout(timer)
  clearTimeout(failedExitTimer)
}
