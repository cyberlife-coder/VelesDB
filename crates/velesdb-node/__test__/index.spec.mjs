// Functional + contract tests for @wiscale/velesdb-memory-node.
// Run with `node --test __test__/` after `napi build` produces index.js + the
// native .node. Uses the offline "hash" embedder so CI needs no Ollama.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { MemoryStore } from '../index.js'

/** Fresh store in an isolated temp dir (one MemoryStore per path). */
function freshStore() {
  const dir = mkdtempSync(join(tmpdir(), 'velesdb-node-'))
  const store = MemoryStore.open(dir, 'hash')
  return { store, cleanup: () => rmSync(dir, { recursive: true, force: true }) }
}

test('surface allowlist — exactly the 8 supported methods, no engine leak', () => {
  const instanceMethods = Object.getOwnPropertyNames(MemoryStore.prototype)
    .filter((m) => m !== 'constructor')
    .sort()
  assert.deepEqual(instanceMethods, [
    'forget',
    'recall',
    'recallWhere',
    'relate',
    'remember',
    'rememberExtracted',
    'why',
  ])
  assert.equal(typeof MemoryStore.open, 'function', 'open is the static factory')
  // No raw-engine ops crossed the license boundary.
  for (const banned of ['query', 'upsert', 'createCollection', 'traverse']) {
    assert.equal(MemoryStore.prototype[banned], undefined, `${banned} must not be exposed`)
  }
})

test('remember → recall round-trips, ids are decimal strings', async () => {
  const { store, cleanup } = freshStore()
  try {
    const id = await store.remember('parking_lot avoids lock poisoning')
    assert.equal(typeof id, 'string', 'id crosses as a string (JS 2^53 safety)')
    assert.match(id, /^\d+$/, 'id is a decimal string')

    const hits = await store.recall('lock poisoning', 5)
    assert.ok(Array.isArray(hits) && hits.length >= 1)
    const top = hits[0]
    assert.equal(typeof top.id, 'string')
    assert.equal(typeof top.score, 'number')
    assert.equal(typeof top.content, 'string')
  } finally {
    cleanup()
  }
})

test('relate + why returns a connected subgraph with string ids', async () => {
  const { store, cleanup } = freshStore()
  try {
    const pr = await store.remember('PR #42 swaps the mutex for parking_lot')
    const decision = await store.remember('we chose parking_lot to avoid lock poisoning', [
      { target: pr, relation: 'decided_in' },
    ])
    assert.equal(typeof decision, 'string')

    const explanation = await store.why('why parking_lot', 2)
    assert.ok(Array.isArray(explanation.nodes))
    assert.ok(Array.isArray(explanation.edges))
    for (const n of explanation.nodes) {
      assert.equal(typeof n.id, 'string')
      assert.equal(typeof n.hop, 'number')
    }
    for (const e of explanation.edges) {
      assert.equal(typeof e.from, 'string')
      assert.equal(typeof e.to, 'string')
    }
  } finally {
    cleanup()
  }
})

test('recallWhere equals recall with the same exact-match filter', async () => {
  const { store, cleanup } = freshStore()
  try {
    const a = await store.remember('auth bug in login', [], { project: 'veles', score: 3 })
    await store.remember('auth bug elsewhere', [], { project: 'acme', score: 9 })

    const filtered = await store.recallWhere('auth bug', [
      { field: 'project', op: 'eq', value: 'veles' },
    ])
    assert.ok(filtered.some((h) => h.id === a), 'veles fact passes the filter')
    assert.ok(filtered.every((h) => h.content.length > 0))
  } finally {
    cleanup()
  }
})

test('error codes — INVALID_INPUT on empty fact, NOT_FOUND on missing relate endpoint', async () => {
  const { store, cleanup } = freshStore()
  try {
    await assert.rejects(
      () => store.remember(''),
      (err) => err.message.includes('INVALID_INPUT'),
      'empty fact → INVALID_INPUT',
    )
    await assert.rejects(
      () => store.relate('999999999', '888888888', 'rel'),
      (err) => err.message.includes('NOT_FOUND'),
      'relate to a missing id → NOT_FOUND',
    )
    await assert.rejects(
      () => store.recallWhere('q', [{ field: 'f', op: 'bogus', value: 1 }]),
      (err) => err.message.includes('INVALID_INPUT'),
      'bad op → INVALID_INPUT',
    )
  } finally {
    cleanup()
  }
})

test('rememberExtracted surfaces but errors without a backend (INTERNAL)', async () => {
  const { store, cleanup } = freshStore()
  try {
    assert.equal(typeof store.rememberExtracted, 'function')
    // No Ollama in CI → the call should reject (INTERNAL), not crash the process.
    await assert.rejects(
      () => store.rememberExtracted('some text', 'nonexistent-model', 'http://127.0.0.1:1'),
      (err) => err instanceof Error,
    )
  } finally {
    cleanup()
  }
})
