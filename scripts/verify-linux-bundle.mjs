#!/usr/bin/env node
import { spawn } from 'node:child_process'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

import {
  verifyInstalledLinuxRoot,
  verifyPreparedLinuxSidecars,
} from './linux-bundle-layout.mjs'

function usage() {
  return [
    'usage: node scripts/verify-linux-bundle.mjs --target <triple>',
    '       [--sidecars <directory>] [--root <extracted-root>]',
    '       [bundle.deb | bundle.AppImage ...]',
  ].join('\n')
}

function parseArgs(args) {
  const options = { artifacts: [] }
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]
    if (value === '--') continue
    if (value === '--target') options.target = args[++index]
    else if (value === '--sidecars') options.sidecars = args[++index]
    else if (value === '--root') options.root = args[++index]
    else if (value.startsWith('-')) throw new Error(`Unknown option: ${value}`)
    else options.artifacts.push(value)
  }
  if (!options.target) throw new Error('--target is required')
  if (!options.sidecars && !options.root && options.artifacts.length === 0) {
    throw new Error(
      'Provide --sidecars, --root, or at least one bundle artifact',
    )
  }
  return options
}

async function run(command, args, cwd) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, { cwd, stdio: 'inherit' })
    child.once('error', reject)
    child.once('exit', (code, signal) => {
      if (code === 0) resolve()
      else
        reject(new Error(`${command} failed (code=${code}, signal=${signal})`))
    })
  })
}

async function verifyDeb(filePath) {
  const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'tt-complete-deb-'))
  try {
    await run('dpkg-deb', ['-x', path.resolve(filePath), temporary])
    await verifyInstalledLinuxRoot(temporary)
  } finally {
    await fs.rm(temporary, { recursive: true, force: true })
  }
}

async function verifyAppImage(filePath) {
  const temporary = await fs.mkdtemp(
    path.join(os.tmpdir(), 'tt-complete-appimage-'),
  )
  try {
    await run(path.resolve(filePath), ['--appimage-extract'], temporary)
    await verifyInstalledLinuxRoot(path.join(temporary, 'squashfs-root'))
  } finally {
    await fs.rm(temporary, { recursive: true, force: true })
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.sidecars) {
    await verifyPreparedLinuxSidecars(options.sidecars, options.target)
    console.log(`Verified prepared sidecars for ${options.target}`)
  }
  if (options.root) {
    await verifyInstalledLinuxRoot(options.root)
    console.log(`Verified extracted Linux root: ${options.root}`)
  }
  for (const artifact of options.artifacts) {
    if (artifact.endsWith('.deb')) await verifyDeb(artifact)
    else if (artifact.endsWith('.AppImage')) await verifyAppImage(artifact)
    else throw new Error(`Unsupported Linux bundle artifact: ${artifact}`)
    console.log(`Verified Linux bundle: ${artifact}`)
  }
}

main().catch((error) => {
  console.error(error.message)
  console.error(usage())
  process.exitCode = 1
})
