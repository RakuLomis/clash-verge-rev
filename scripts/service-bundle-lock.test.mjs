import assert from 'node:assert/strict'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

import {
  loadServiceBundleLock,
  resolveServiceBundleAsset,
  serviceBundleStampMatches,
  validateServiceBundleLock,
  verifyServiceArchive,
} from './service-bundle-lock.mjs'

const here = path.dirname(fileURLToPath(import.meta.url))
const lockPath = path.join(here, 'service-bundle.lock.json')

test('service bundle lock pins the v2.6.1 Linux IPC contract', async () => {
  const lock = await loadServiceBundleLock(lockPath)
  assert.equal(lock.tag, 'v2.6.1')
  assert.equal(lock.commit, '10edb52ec071520f14ce78654e401e4246127d75')
  assert.equal(lock.protocol.epoch, 2)
  assert.equal(lock.protocol.revision, 2)
  assert.equal(lock.ipcPaths.linux, '/run/clash-verge-service/service.sock')
  assert.equal(Object.keys(lock.assets).length, 9)
})

test('service asset resolution is deterministic and rejects unsupported targets', async () => {
  const lock = await loadServiceBundleLock(lockPath)
  const asset = resolveServiceBundleAsset(lock, 'x86_64-unknown-linux-gnu')
  assert.equal(
    asset.sha256,
    '67eacca41c842d446d848f8fbee99b625906e3a94146cd12e0561c6091a8c0b2',
  )
  assert.equal(
    asset.downloadURL,
    `${lock.source}/releases/download/v2.6.1/${asset.file}`,
  )
  assert.throws(
    () => resolveServiceBundleAsset(lock, 'riscv64gc-unknown-linux-gnu'),
    /No pinned/,
  )
})

test('service archive verification fails closed', () => {
  const data = Buffer.from('known archive bytes')
  const asset = { file: 'service.tar.gz', sha256: '0'.repeat(64) }
  assert.throws(() => verifyServiceArchive(data, asset), /SHA-256 mismatch/)
})

test('service lock validation rejects malformed hashes', async () => {
  const lock = await loadServiceBundleLock(lockPath)
  const invalid = structuredClone(lock)
  invalid.assets['x86_64-unknown-linux-gnu'].sha256 = 'latest'
  assert.throws(
    () => validateServiceBundleLock(invalid),
    /Invalid service asset SHA-256/,
  )
})

test('service sidecars are reusable only with the exact lock stamp and binary hashes', () => {
  const asset = {
    tag: 'v2.6.1',
    commit: '1'.repeat(40),
    target: 'x86_64-unknown-linux-gnu',
    sha256: '2'.repeat(64),
  }
  const binaryHashes = {
    'clash-verge-service-x86_64-unknown-linux-gnu': '3'.repeat(64),
  }
  const stamp = {
    tag: asset.tag,
    commit: asset.commit,
    target: asset.target,
    archiveSha256: asset.sha256,
    binaries: structuredClone(binaryHashes),
  }
  assert.equal(serviceBundleStampMatches(stamp, asset, binaryHashes), true)
  stamp.tag = 'v2.3.0'
  assert.equal(serviceBundleStampMatches(stamp, asset, binaryHashes), false)
  stamp.tag = asset.tag
  stamp.binaries[Object.keys(binaryHashes)[0]] = '4'.repeat(64)
  assert.equal(serviceBundleStampMatches(stamp, asset, binaryHashes), false)
})
