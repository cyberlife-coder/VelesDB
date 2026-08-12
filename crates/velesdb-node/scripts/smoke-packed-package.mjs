import assert from 'node:assert/strict'
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

const REQUIRED_ROOT_FILES = ['index.js', 'index.d.ts']

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8' })
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(' ')} failed (${result.status})\n${result.stdout}${result.stderr}`,
    )
  }
  return result.stdout
}

function pack(packageDir, destination) {
  mkdirSync(destination)
  const output = run('npm', ['pack', '--json', '--pack-destination', destination], packageDir)
  const reports = JSON.parse(output)
  assert.equal(reports.length, 1, `expected one packed artifact, got ${reports.length}`)
  return { report: reports[0], tarball: join(destination, reports[0].filename) }
}

function assertRootEntries(report) {
  const paths = new Set(report.files.map(({ path }) => path))
  for (const file of REQUIRED_ROOT_FILES) {
    assert.ok(paths.has(file), `packed root package is missing ${file}`)
  }
}

function installAndRequire(consumerDir, rootTarball, platformTarball) {
  mkdirSync(consumerDir)
  writeFileSync(join(consumerDir, 'package.json'), '{"private":true}\n')
  run(
    'npm',
    [
      'install',
      '--offline',
      '--ignore-scripts',
      '--no-audit',
      '--no-fund',
      rootTarball,
      platformTarball,
    ],
    consumerDir,
  )

  const installedRoot = join(
    consumerDir,
    'node_modules',
    '@wiscale',
    'velesdb-memory-node',
  )
  for (const file of REQUIRED_ROOT_FILES) {
    assert.ok(existsSync(join(installedRoot, file)), `fresh install is missing ${file}`)
  }
  run(
    process.execPath,
    [
      '-e',
      "const addon = require('@wiscale/velesdb-memory-node'); " +
        "if (typeof addon.MemoryService?.open !== 'function') " +
        "throw new Error('MemoryService.open is missing')",
    ],
    consumerDir,
  )
}

const platformPackageDir = process.argv[2]
assert.ok(platformPackageDir, 'usage: smoke-packed-package.mjs <platform-package-dir>')

const packageRoot = process.cwd()
const platformRoot = resolve(packageRoot, platformPackageDir)

const scratch = mkdtempSync(join(tmpdir(), 'velesdb-node-pack-'))
try {
  const root = pack(packageRoot, join(scratch, 'root'))
  assertRootEntries(root.report)
  assert.ok(existsSync(platformRoot), `platform package directory not found: ${platformRoot}`)
  const platform = pack(platformRoot, join(scratch, 'platform'))
  installAndRequire(join(scratch, 'consumer'), root.tarball, platform.tarball)
  console.log('packed root package installs and loads from a clean directory')
} finally {
  rmSync(scratch, { recursive: true, force: true })
}
