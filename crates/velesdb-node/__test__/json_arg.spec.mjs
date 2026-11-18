// Contract tests for the one conversion of a JS argument into JSON data
// (src/json_arg.rs): metadata, filters, compile requests, working contexts.
// Run with `node --test __test__/` after `napi build`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { MemoryService } from '../index.js'

process.env.VELESDB_MEMORY_QUIET = '1'

const INDEX = fileURLToPath(new URL('../index.js', import.meta.url))

function freshStore() {
  const dir = mkdtempSync(join(tmpdir(), 'velesdb-node-json-'))
  const store = MemoryService.open(dir, 'hash')
  return {
    store,
    cleanup: () => {
      try {
        rmSync(dir, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 })
      } catch {}
    },
  }
}

/** Await `promise` and return its rejection message (fails if it resolves). */
async function rejection(promise) {
  try {
    await promise
  } catch (err) {
    return String(err.message)
  }
  assert.fail('expected a rejection')
}

test('an array whose length is far past any argument rejects instead of aborting the process', () => {
  // A sparse array of length 2^32 - 1 costs nothing to build in JS. The walk
  // used to pre-allocate one slot per claimed element, so the process died
  // on a failed allocation (SIGABRT) before any validation: run it in a child.
  const script = `
    process.env.VELESDB_MEMORY_QUIET = '1'
    const { mkdtempSync } = require('node:fs')
    const { join } = require('node:path')
    const { tmpdir } = require('node:os')
    const { MemoryService } = require(${JSON.stringify(INDEX)})
    const store = MemoryService.open(mkdtempSync(join(tmpdir(), 'velesdb-node-huge-')), 'hash')
    const huge = []
    huge.length = 2 ** 32 - 1
    const calls = [
      () => store.remember('fact', [], { huge }),
      () => store.recall('fact', 5, { huge }),
      () => store.compileContext({ fragments: huge, budget_tokens: 10 }),
      () => store.saveWorkingContext('p', 's', { decisions: huge }),
      () => store.recallWhere('fact', [{ field: 'x', op: 'eq', value: huge }]),
      () => store.recallWhere('fact', huge),
    ]
    ;(async () => {
      for (const call of calls) {
        try {
          await call()
          console.log('resolved')
        } catch (err) {
          console.log(err.message.startsWith('[INVALID_INPUT]') ? 'refused' : 'other: ' + err.message)
        }
      }
    })()
  `
  const child = spawnSync(process.execPath, ['-e', script], { encoding: 'utf8' })
  assert.equal(child.status, 0, `child died: ${child.stderr}`)
  assert.deepEqual(child.stdout.trim().split('\n'), Array(6).fill('refused'))
})

test('only own enumerable properties are read, as JSON.stringify reads them', async () => {
  const { store, cleanup } = freshStore()
  Object.prototype.inheritedLeak = 'yes'
  try {
    await store.remember('beta fact', [], { b: 2 })
  } finally {
    delete Object.prototype.inheritedLeak
  }
  try {
    const listed = (await store.listMemories(null, 50, null, false)).memories
    const fact = listed.find((m) => m.content === 'beta fact')
    assert.ok(fact, 'the fact is listed')
    assert.equal(fact.metadata.b, 2)
    assert.equal('inheritedLeak' in fact.metadata, false, 'an inherited key is not stored')
  } finally {
    cleanup()
  }
})

test('a BigInt round-trips: the integer read back as a BigInt filters on that integer', async () => {
  const { store, cleanup } = freshStore()
  try {
    const serial = 2n ** 60n
    await store.remember('serial fact', [], { serial })
    const listed = (await store.listMemories(null, 50, { serial }, false)).memories
    assert.deepEqual(
      listed.map((m) => m.content),
      ['serial fact'],
    )
    assert.equal(listed[0].metadata.serial, serial, 'read back as the same BigInt')
    // A negative one, and the edges of the 64-bit range JSON metadata holds.
    await store.remember('edges', [], { min: -(2n ** 63n), max: 2n ** 64n - 1n })
    const edges = ((await store.listMemories(null, 50, null, false)).memories).find((m) => m.content === 'edges')
    assert.equal(edges.metadata.min, -(2n ** 63n))
    assert.equal(edges.metadata.max, 2n ** 64n - 1n)
  } finally {
    cleanup()
  }
})

test('a BigInt outside the 64-bit range is refused with INVALID_INPUT', async () => {
  const { store, cleanup } = freshStore()
  try {
    const msg = await rejection(store.remember('f', [], { big: 2n ** 64n }))
    assert.match(msg, /^\[INVALID_INPUT\] metadata\.big /)
    const neg = await rejection(store.remember('f', [], { big: -(2n ** 63n) - 1n }))
    assert.match(neg, /^\[INVALID_INPUT\] metadata\.big /)
  } finally {
    cleanup()
  }
})

test('a key holding a NUL character is read like any other key', async () => {
  const { store, cleanup } = freshStore()
  try {
    await store.remember('nul key fact', [], { 'a\u0000b': 1 })
    const fact = ((await store.listMemories(null, 50, null, false)).memories).find((m) => m.content === 'nul key fact')
    assert.equal(fact.metadata['a\u0000b'], 1)
  } finally {
    cleanup()
  }
})

test('recallWhere filters go through the same conversion: malformed ones reject with INVALID_INPUT', async () => {
  const { store, cleanup } = freshStore()
  try {
    const cases = [
      [{ field: 'x', op: 'eq' }],
      [{ field: 'x', op: 'eq', value: undefined }],
      [{ field: 'x', op: 'eq', value: new Date(0) }],
      [{ field: 'x', op: 'eq', value: new Map() }],
      [{ field: 1, op: 'eq', value: 1 }],
      'not an array',
    ]
    for (const filters of cases) {
      const msg = await rejection(store.recallWhere('q', filters))
      assert.match(msg, /^\[INVALID_INPUT\] /, `filters ${String(filters)} → ${msg}`)
    }
  } finally {
    cleanup()
  }
})

test('recallWhere compares against a BigInt the same as metadata stores it', async () => {
  const { store, cleanup } = freshStore()
  try {
    await store.remember('serial fact', [], { serial: 2n ** 60n })
    const hits = await store.recallWhere('serial fact', [{ field: 'serial', op: 'eq', value: 2n ** 60n }])
    assert.deepEqual(
      hits.map((h) => h.content),
      ['serial fact'],
    )
  } finally {
    cleanup()
  }
})
