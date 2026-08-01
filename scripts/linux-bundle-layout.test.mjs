import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'

import {
  requiredLinuxBundleBinaries,
  verifyInstalledLinuxRoot,
  verifyPreparedLinuxSidecars,
} from './linux-bundle-layout.mjs'

const target = 'x86_64-unknown-linux-gnu'

async function fixture(run) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'tt-linux-bundle-'))
  try {
    await run(root)
  } finally {
    await fs.rm(root, { recursive: true, force: true })
  }
}

async function executable(filePath) {
  await fs.mkdir(path.dirname(filePath), { recursive: true })
  await fs.writeFile(filePath, 'fixture')
  await fs.chmod(filePath, 0o755)
}

test('accepts all executable prepared TrafficTracer Complete sidecars', async () => {
  await fixture(async (root) => {
    for (const binary of requiredLinuxBundleBinaries) {
      await executable(path.join(root, `${binary}-${target}`))
    }
    await verifyPreparedLinuxSidecars(root, target)
  })
})

test('accepts the installed deb and AppImage usr/bin layout', async () => {
  await fixture(async (root) => {
    for (const binary of requiredLinuxBundleBinaries) {
      await executable(path.join(root, 'usr', 'bin', binary))
    }
    assert.deepEqual(
      await verifyInstalledLinuxRoot(root),
      requiredLinuxBundleBinaries,
    )
  })
})

test('rejects a bundle that omits the TrafficTracer core', async () => {
  await fixture(async (root) => {
    for (const binary of requiredLinuxBundleBinaries) {
      if (binary !== 'verge-mihomo-tt') {
        await executable(path.join(root, 'usr', 'bin', binary))
      }
    }
    await assert.rejects(verifyInstalledLinuxRoot(root), /verge-mihomo-tt/)
  })
})

test('rejects a non-executable Worker', async () => {
  await fixture(async (root) => {
    for (const binary of requiredLinuxBundleBinaries) {
      await executable(path.join(root, 'usr', 'bin', binary))
    }
    await fs.chmod(path.join(root, 'usr', 'bin', 'traffictracer-worker'), 0o644)
    await assert.rejects(
      verifyInstalledLinuxRoot(root),
      /not executable: .*traffictracer-worker/,
    )
  })
})
