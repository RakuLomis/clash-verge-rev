// Run only the isolated Cargo example, never the production application.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'

const binary = resolve('target/debug/examples/traffictracer-ui-soak')
const cases = [
  { name: 'normal', fault: '', delay: '0', status: 'passed', code: 0 },
  {
    name: 'sampling-delay',
    fault: '',
    delay: '800',
    status: 'passed',
    code: 0,
  },
  {
    name: 'event-stall',
    fault: 'event_stall',
    delay: '0',
    status: 'failed',
    code: 1,
  },
  {
    name: 'early-close',
    fault: 'early_close',
    delay: '0',
    status: 'interrupted',
    code: 1,
  },
]
for (const scenario of cases) {
  let output = ''
  let timedOut = false
  const child = spawn(binary, ['20'], {
    env: {
      ...process.env,
      TT_SOAK_FAULT: scenario.fault,
      TT_SOAK_SAMPLE_DELAY_MS: scenario.delay,
    },
    stdio: ['ignore', 'pipe', 'inherit'],
  })
  child.stdout.on('data', (chunk) => {
    output += chunk.toString()
  })
  // Bound failure testing without touching unrelated processes.
  const timer = setTimeout(() => {
    timedOut = true
    child.kill('SIGKILL')
  }, 45_000)
  let exit
  try {
    exit = await new Promise((resolveExit, reject) => {
      child.once('error', reject)
      child.once('close', (code, signal) => resolveExit({ code, signal }))
    })
  } finally {
    clearTimeout(timer)
  }
  assert.equal(timedOut, false, `${scenario.name}: timeout`)
  assert.equal(exit.signal, null, `${scenario.name}: abnormal exit`)
  const directory = output.match(/^SOAK_REPORT_DIR=(.+)$/m)?.[1]
  assert.ok(directory, `${scenario.name}: missing report location`)
  const report = JSON.parse(
    await readFile(resolve(directory, 'result.json'), 'utf8'),
  )
  assert.equal(report.status, scenario.status, scenario.name)
  assert.equal(report.passed, scenario.code === 0, scenario.name)
  assert.equal(
    exit.code,
    scenario.code,
    `${scenario.name}: exit/report mismatch`,
  )
  if (scenario.fault === 'event_stall')
    assert.equal(report.first_failure.reason, 'EVENT_STALLED')
  if (scenario.code === 0) {
    assert.ok(report.wall_elapsed_ms >= 20_000)
    assert.ok(
      report.wall_elapsed_ms < 25_000,
      'sampling overhead extended the deadline',
    )
  }
  console.log(
    JSON.stringify({
      scenario: scenario.name,
      status: report.status,
      exit: exit.code,
      directory,
    }),
  )
}
