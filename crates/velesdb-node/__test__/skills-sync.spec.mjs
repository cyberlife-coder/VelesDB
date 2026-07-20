// Guards the skills bundled into the @wiscale/velesdb-memory-node npm package
// (package.json "files": ["skills/"]) against silent drift from their
// sources of truth elsewhere in the repo. There is no build step, symlink,
// or CI check that keeps these copies in sync — they are committed,
// hand-copied duplicates (velesdb-memory added in PR #1496) — so this test
// is the only thing that notices when a source SKILL.md changes but the
// bundled copy does not.
//
// Pure Node fs, no napi addon needed: `node --test __test__/skills-sync.spec.mjs`
// runs standalone without `napi build` first.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { join, relative } from 'node:path'

// Resolved from import.meta.url (not cwd) so this test works regardless of
// where `node --test` is invoked from (repo root, crates/velesdb-node/, CI
// working-directory, etc.).
const NODE_SKILLS_DIR = fileURLToPath(new URL('../skills', import.meta.url))
const REPO_ROOT = fileURLToPath(new URL('../../..', import.meta.url))

/** Recursively list files under `dir`, returned as repo-relative-to-`dir` POSIX paths. */
function listFiles(dir) {
  const out = []
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const full = join(current, entry.name)
      if (entry.isDirectory()) {
        walk(full)
      } else if (entry.isFile()) {
        out.push(relative(dir, full).split('\\').join('/'))
      }
    }
  }
  walk(dir)
  return out.sort()
}

// (source dir, bundled copy dir) pairs. Add new bundled skills here.
const PAIRS = [
  {
    name: 'velesdb-context-optimizer',
    source: join(REPO_ROOT, 'skills', 'velesdb-context-optimizer'),
    copy: join(NODE_SKILLS_DIR, 'velesdb-context-optimizer'),
  },
  {
    name: 'velesdb-memory',
    source: join(REPO_ROOT, 'crates', 'velesdb-memory', 'skill', 'velesdb-memory'),
    copy: join(NODE_SKILLS_DIR, 'velesdb-memory'),
  },
]

for (const { name, source, copy } of PAIRS) {
  test(`bundled skill "${name}" stays byte-identical to its source`, () => {
    assert.ok(
      statSync(source, { throwIfNoEntry: false })?.isDirectory(),
      `source dir missing: ${source}`,
    )
    assert.ok(
      statSync(copy, { throwIfNoEntry: false })?.isDirectory(),
      `bundled copy dir missing: ${copy}`,
    )

    const sourceFiles = listFiles(source)
    const copyFiles = listFiles(copy)

    const resync = `cp -r ${relative(REPO_ROOT, source)}/. ${relative(REPO_ROOT, copy)}/`

    const onlyInSource = sourceFiles.filter((f) => !copyFiles.includes(f))
    const onlyInCopy = copyFiles.filter((f) => !sourceFiles.includes(f))
    assert.deepEqual(
      onlyInSource,
      [],
      `file(s) present in source but missing from the bundled npm copy: ` +
        `${onlyInSource.join(', ')}. Resync with: ${resync}`,
    )
    assert.deepEqual(
      onlyInCopy,
      [],
      `file(s) present in the bundled npm copy but not in the source (stale/orphaned): ` +
        `${onlyInCopy.join(', ')}. Resync with: ${resync}`,
    )

    const mismatched = []
    for (const relPath of sourceFiles) {
      const sourceContent = readFileSync(join(source, relPath))
      const copyContent = readFileSync(join(copy, relPath))
      if (!sourceContent.equals(copyContent)) {
        mismatched.push(relPath)
      }
    }
    assert.deepEqual(
      mismatched,
      [],
      `bundled npm copy has drifted from its source for: ${mismatched.join(', ')}. ` +
        `Resync with: ${resync}`,
    )
  })
}
