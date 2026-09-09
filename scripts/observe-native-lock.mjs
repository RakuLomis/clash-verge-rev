// Diagnostic only. Never locks the desktop or touches the production process.
import { spawn, execFile } from 'node:child_process'
import { appendFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { performance } from 'node:perf_hooks'

const seconds = Number(process.argv[2] ?? 240)
const session = process.argv[3]
if (
  !Number.isInteger(seconds) ||
  seconds < 60 ||
  seconds > 14400 ||
  !/^[\w-]+$/.test(session ?? '')
)
  throw new Error(
    'Usage: node scripts/observe-native-lock.mjs SECONDS SESSION_ID',
  )
const started = performance.now()
let directory
let output = ''
let pending = false
let stopped = false
let count = 0
const queued = []
const child = spawn(
  'timeout',
  [
    '--signal=TERM',
    '--kill-after=5s',
    `${seconds + 30}s`,
    resolve('target/debug/examples/traffictracer-ui-soak'),
    String(seconds),
  ],
  {
    env: {
      ...process.env,
      TT_SOAK_OBSERVE_UNTIL_DEADLINE: '1',
      TT_SOAK_FAULT: '',
      TT_SOAK_SAMPLE_DELAY_MS: '0',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  },
)
const record = (sample) => {
  const line = `${JSON.stringify(sample)}\n`
  if (directory) appendFileSync(resolve(directory, 'os-session.jsonl'), line)
  else if (queued.length < 30) queued.push(line)
}
const poll = () => {
  if (pending || stopped) return
  pending = true
  execFile(
    'loginctl',
    [
      'show-session',
      session,
      '-p',
      'LockedHint',
      '-p',
      'Active',
      '-p',
      'State',
    ],
    { timeout: 1500, maxBuffer: 4096 },
    (error, stdout) => {
      pending = false
      if (stopped) return
      count++
      record({
        at: new Date().toISOString(),
        elapsed_ms: Math.round(performance.now() - started),
        session,
        values: error
          ? null
          : Object.fromEntries(
              stdout
                .trim()
                .split('\n')
                .map((line) => line.split('=')),
            ),
        error: error ? String(error.message) : null,
      })
    },
  )
}
const remember = (chunk) => {
  output = (output + chunk.toString()).slice(-16384)
  const found = output.match(/^SOAK_REPORT_DIR=([^\r\n]+)/m)?.[1]
  if (!directory && found) {
    directory = found
    writeFileSync(resolve(directory, 'os-session.jsonl'), queued.join(''))
    queued.length = 0
    console.log(
      JSON.stringify({
        status: 'observing',
        directory,
        seconds,
        session,
        at: new Date().toISOString(),
      }),
    )
  }
}
child.stdout.on('data', remember)
child.stderr.on('data', remember)
const timer = setInterval(poll, 2000)
poll()
child.on('error', (error) => {
  stopped = true
  clearInterval(timer)
  console.error(error.message)
  process.exitCode = 1
})
child.on('close', (code, signal) => {
  stopped = true
  clearInterval(timer)
  const summary = {
    code,
    signal,
    seconds,
    session,
    os_samples: count,
    directory,
    output,
  }
  if (directory)
    writeFileSync(
      resolve(directory, 'lock-observer.json'),
      JSON.stringify(summary, null, 2),
    )
  console.log(JSON.stringify(summary))
  // Diagnostic runs deliberately do not qualify as successful stability gates.
  process.exitCode = code ?? 1
})
