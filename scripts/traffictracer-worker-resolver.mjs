import fs from 'fs/promises'
import path from 'path'

const workerName = (targetTriple) => `traffictracer-worker-${targetTriple}`

async function isFile(candidate) {
  if (!candidate) return false

  try {
    return (await fs.stat(candidate)).isFile()
  } catch (error) {
    if (error.code === 'ENOENT') return false
    throw error
  }
}

export function trafficTracerWorkerPaths({
  cwd,
  sidecarDir,
  targetTriple,
  configuredSource,
}) {
  const completeRoot = path.resolve(cwd, '..', '..')
  const fileName = workerName(targetTriple)

  return {
    completeRoot,
    targetPath: path.join(sidecarDir, fileName),
    candidates: [
      configuredSource,
      path.join(completeRoot, 'dist', 'worker', fileName),
    ].filter(Boolean),
  }
}

export async function resolveTrafficTracerWorkerSidecar({
  cwd,
  sidecarDir,
  targetTriple,
  configuredSource,
  force = false,
}) {
  const { completeRoot, targetPath, candidates } = trafficTracerWorkerPaths({
    cwd,
    sidecarDir,
    targetTriple,
    configuredSource,
  })

  if (!force && (await isFile(targetPath))) {
    return { status: 'skipped', targetPath }
  }

  let sourcePath
  for (const candidate of candidates) {
    if (await isFile(candidate)) {
      sourcePath = candidate
      break
    }
  }

  if (!sourcePath) {
    throw new Error(
      `TrafficTracer Worker for ${targetTriple} was not found. ` +
        'Set TRAFFICTRACER_WORKER_BIN to the matching executable, or run ' +
        `cd ${completeRoot} && bash scripts/build-worker.sh`,
    )
  }

  await fs.mkdir(sidecarDir, { recursive: true })
  await fs.copyFile(sourcePath, targetPath)
  await fs.chmod(targetPath, 0o755)
  return { status: 'copied', sourcePath, targetPath }
}
