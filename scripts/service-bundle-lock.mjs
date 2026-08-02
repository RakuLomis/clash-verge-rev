import { createHash } from 'crypto'
import fs from 'fs/promises'

const SHA256_PATTERN = /^[a-f0-9]{64}$/

export async function loadServiceBundleLock(lockPath) {
  const lock = JSON.parse(await fs.readFile(lockPath, 'utf8'))
  validateServiceBundleLock(lock)
  return lock
}

export function validateServiceBundleLock(lock) {
  if (lock?.schemaVersion !== 1)
    throw new Error('Unsupported service bundle lock schema')
  if (!/^v\d+\.\d+\.\d+$/.test(lock.tag ?? ''))
    throw new Error('Invalid service release tag')
  if (!/^[a-f0-9]{40}$/.test(lock.commit ?? ''))
    throw new Error('Invalid service source commit')
  if (!lock.source?.startsWith('https://github.com/'))
    throw new Error('Invalid service source URL')
  if (!lock.assets || Object.keys(lock.assets).length === 0)
    throw new Error('Service lock has no assets')
  for (const [target, asset] of Object.entries(lock.assets)) {
    if (!asset.file?.includes(`${lock.tag}-${target}.`)) {
      throw new Error(`Service asset name does not match ${target}`)
    }
    if (!SHA256_PATTERN.test(asset.sha256 ?? '')) {
      throw new Error(`Invalid service asset SHA-256 for ${target}`)
    }
  }
  return lock
}

export function resolveServiceBundleAsset(lock, target) {
  validateServiceBundleLock(lock)
  const asset = lock.assets[target]
  if (!asset) {
    throw new Error(
      `No pinned clash-verge-service-ipc ${lock.tag} asset for Rust target ${target}`,
    )
  }
  return {
    ...asset,
    target,
    tag: lock.tag,
    commit: lock.commit,
    downloadURL: `${lock.source}/releases/download/${lock.tag}/${asset.file}`,
  }
}

export function sha256(data) {
  return createHash('sha256').update(data).digest('hex')
}

export function verifyServiceArchive(data, asset) {
  const actual = sha256(data)
  if (actual !== asset.sha256) {
    throw new Error(
      `Service archive SHA-256 mismatch for ${asset.file}: expected ${asset.sha256}, got ${actual}`,
    )
  }
  return actual
}

export function serviceBundleStampMatches(stamp, asset, binaryHashes) {
  return (
    stamp?.tag === asset.tag &&
    stamp?.commit === asset.commit &&
    stamp?.target === asset.target &&
    stamp?.archiveSha256 === asset.sha256 &&
    Object.entries(binaryHashes).every(
      ([file, hash]) =>
        typeof hash === 'string' && stamp.binaries?.[file] === hash,
    )
  )
}
