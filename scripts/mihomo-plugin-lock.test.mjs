import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const revisionPattern = '[0-9a-f]{40}'
const compatibleRevision = 'cf97ff99e390a9b437d5cf94c6f454f024fc8f69'

test('Rust and JavaScript Mihomo plugins pin the same revision', async () => {
  const [packageJsonText, cargoToml, cargoLock, pnpmLock] = await Promise.all([
    fs.readFile(path.join(root, 'package.json'), 'utf8'),
    fs.readFile(path.join(root, 'src-tauri', 'Cargo.toml'), 'utf8'),
    fs.readFile(path.join(root, 'Cargo.lock'), 'utf8'),
    fs.readFile(path.join(root, 'pnpm-lock.yaml'), 'utf8'),
  ])
  const packageJson = JSON.parse(packageJsonText)
  const jsSpecifier = packageJson.dependencies['tauri-plugin-mihomo-api']
  const jsRevision = jsSpecifier.match(
    new RegExp('#(' + revisionPattern + ')$'),
  )?.[1]
  const rustRevision = cargoToml.match(
    new RegExp(
      'tauri-plugin-mihomo\\s*=\\s*\\{[^\\n]*rev\\s*=\\s*"(' +
        revisionPattern +
        ')"',
    ),
  )?.[1]

  assert.ok(jsRevision, 'JavaScript Mihomo plugin must pin a full revision')
  assert.ok(rustRevision, 'Rust Mihomo plugin must pin a full revision')
  assert.equal(jsRevision, rustRevision)
  assert.equal(
    rustRevision,
    compatibleRevision,
    'Mihomo plugin revision must include tolerant response DTO defaults',
  )
  assert.match(
    cargoLock,
    new RegExp(
      'tauri-plugin-mihomo\\?rev=' + rustRevision + '#' + rustRevision,
    ),
  )
  assert.match(
    pnpmLock,
    new RegExp('tauri-plugin-mihomo/tar\\.gz/' + jsRevision),
  )
})
