import fs from 'node:fs/promises'
import path from 'node:path'

export const requiredLinuxBundleBinaries = Object.freeze([
  'clash-verge-service',
  'clash-verge-service-install',
  'clash-verge-service-uninstall',
  'verge-mihomo',
  'verge-mihomo-alpha',
  'verge-mihomo-tt',
  'traffictracer-worker',
])

async function assertExecutable(filePath, label) {
  const stat = await fs.stat(filePath).catch(() => null)
  if (!stat?.isFile() || stat.size === 0) {
    throw new Error(`${label} is missing or empty: ${filePath}`)
  }
  if ((stat.mode & 0o111) === 0) {
    throw new Error(`${label} is not executable: ${filePath}`)
  }
}

export async function verifyPreparedLinuxSidecars(sidecarDir, targetTriple) {
  for (const binary of requiredLinuxBundleBinaries) {
    await assertExecutable(
      path.join(sidecarDir, `${binary}-${targetTriple}`),
      'Prepared sidecar',
    )
  }
}

export async function verifyInstalledLinuxRoot(rootDir) {
  const binDir = path.join(rootDir, 'usr', 'bin')
  for (const binary of requiredLinuxBundleBinaries) {
    await assertExecutable(path.join(binDir, binary), 'Bundled executable')
  }
  return requiredLinuxBundleBinaries
}
