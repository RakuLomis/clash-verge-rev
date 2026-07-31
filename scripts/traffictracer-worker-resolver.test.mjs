import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

import { resolveTrafficTracerWorkerSidecar } from './traffictracer-worker-resolver.mjs'

const targetTriple = 'x86_64-unknown-linux-gnu'
const fileName = `traffictracer-worker-${targetTriple}`

async function fixture(run) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'tt-worker-resolver-'))
  const cwd = path.join(root, 'components', 'clash-verge-rev')
  const sidecarDir = path.join(cwd, 'src-tauri', 'sidecar')
  await fs.mkdir(cwd, { recursive: true })

  try {
    await run({ root, cwd, sidecarDir })
  } finally {
    await fs.rm(root, { recursive: true, force: true })
  }
}

test('copies the worker from the Complete dist directory by default', async () => {
  await fixture(async ({ root, cwd, sidecarDir }) => {
    const source = path.join(root, 'dist', 'worker', fileName)
    await fs.mkdir(path.dirname(source), { recursive: true })
    await fs.writeFile(source, 'worker-v1')

    const result = await resolveTrafficTracerWorkerSidecar({
      cwd,
      sidecarDir,
      targetTriple,
    })

    assert.equal(result.status, 'copied')
    assert.equal(await fs.readFile(result.targetPath, 'utf8'), 'worker-v1')
    assert.equal((await fs.stat(result.targetPath)).mode & 0o777, 0o755)
  })
})

test('uses TRAFFICTRACER_WORKER_BIN when configured', async () => {
  await fixture(async ({ root, cwd, sidecarDir }) => {
    const configuredSource = path.join(root, 'custom-worker')
    await fs.writeFile(configuredSource, 'custom')

    const result = await resolveTrafficTracerWorkerSidecar({
      cwd,
      sidecarDir,
      targetTriple,
      configuredSource,
    })

    assert.equal(result.sourcePath, configuredSource)
    assert.equal(await fs.readFile(result.targetPath, 'utf8'), 'custom')
  })
})

test('force overwrites a stale sidecar while the default skips it', async () => {
  await fixture(async ({ root, cwd, sidecarDir }) => {
    const source = path.join(root, 'dist', 'worker', fileName)
    const target = path.join(sidecarDir, fileName)
    await fs.mkdir(path.dirname(source), { recursive: true })
    await fs.mkdir(sidecarDir, { recursive: true })
    await fs.writeFile(source, 'fresh')
    await fs.writeFile(target, 'stale')

    const skipped = await resolveTrafficTracerWorkerSidecar({
      cwd,
      sidecarDir,
      targetTriple,
    })
    assert.equal(skipped.status, 'skipped')
    assert.equal(await fs.readFile(target, 'utf8'), 'stale')

    const copied = await resolveTrafficTracerWorkerSidecar({
      cwd,
      sidecarDir,
      targetTriple,
      force: true,
    })
    assert.equal(copied.status, 'copied')
    assert.equal(await fs.readFile(target, 'utf8'), 'fresh')
  })
})

test('missing worker error includes the target and recovery command', async () => {
  await fixture(async ({ cwd, sidecarDir }) => {
    await assert.rejects(
      resolveTrafficTracerWorkerSidecar({
        cwd,
        sidecarDir,
        targetTriple: 'aarch64-unknown-linux-gnu',
      }),
      (error) => {
        assert.match(error.message, /aarch64-unknown-linux-gnu/)
        assert.match(error.message, /TRAFFICTRACER_WORKER_BIN/)
        assert.match(error.message, /bash scripts\/build-worker\.sh/)
        return true
      },
    )
  })
})
