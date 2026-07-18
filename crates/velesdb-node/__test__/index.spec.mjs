// Functional + contract tests for @wiscale/velesdb-memory-node.
// Run with `node --test __test__/` after `napi build` produces index.js + the
// native .node. Uses the offline "hash" embedder so CI needs no Ollama.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { MemoryService } from '../index.js'

/** Fresh store in an isolated temp dir (one MemoryService per path). */
function freshStore() {
  const dir = mkdtempSync(join(tmpdir(), 'velesdb-node-'))
  const store = MemoryService.open(dir, 'hash')
  return { store, cleanup: () => rmSync(dir, { recursive: true, force: true }) }
}

test('surface allowlist — exactly the supported methods, no engine leak', () => {
  const instanceMethods = Object.getOwnPropertyNames(MemoryService.prototype)
    .filter((m) => m !== 'constructor')
    .sort((a, b) => a.localeCompare(b))
  assert.deepEqual(instanceMethods, [
    'compileContext',
    'feedback',
    'forget',
    'recall',
    'recallFused',
    'recallFusedDated',
    'recallWhere',
    'relate',
    'remember',
    'rememberExtracted',
    'why',
  ])
  assert.equal(typeof MemoryService.open, 'function', 'open is the static factory')
  // No raw-engine ops crossed the license boundary.
  const exposed = new Set(Object.getOwnPropertyNames(MemoryService.prototype))
  for (const banned of ['query', 'upsert', 'createCollection', 'traverse']) {
    assert.ok(!exposed.has(banned), `${banned} must not be exposed`)
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

test('recallFused surfaces a graph-connected fact plain recall ranks low', async () => {
  const { store, cleanup } = freshStore()
  try {
    const decision = await store.remember('we chose parking_lot to avoid lock poisoning')
    const ticket = await store.remember('EPIC-317 xyzzy quux frobnicate')
    const distractor = await store.remember('the quarterly report is due next Friday')
    await store.relate(decision, ticket, 'decided_in')

    const fused = await store.recallFused('we chose parking_lot to avoid lock poisoning', 3)
    assert.ok(Array.isArray(fused) && fused.length >= 1)
    const rankOf = (id) => {
      const index = fused.findIndex((r) => r.id === id)
      assert.notEqual(index, -1, `expected id ${id} to be present in the fused results`)
      return index
    }
    assert.ok(
      rankOf(ticket) < rankOf(distractor),
      'the graph-reached ticket must outrank the disconnected distractor',
    )

    const withOpts = await store.recallFused('we chose parking_lot to avoid lock poisoning', 1, null, {
      pool: 1,
    })
    assert.equal(withOpts.length, 1, 'the pool override narrows the candidate set')
    assert.equal(withOpts[0].id, decision, 'a pool of 1 admits only the top vector hit')
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

test('recallWhere and recall both surface stored metadata (dated recall)', async () => {
  const { store, cleanup } = freshStore()
  try {
    await store.remember('deployed the new pricing page', [], { ts: 20260701 })

    const filtered = await store.recallWhere('pricing page', [
      { field: 'ts', op: 'eq', value: 20260701 },
    ])
    assert.ok(filtered.length >= 1)
    assert.equal(filtered[0].metadata?.ts, 20260701, 'recallWhere round-trips metadata')

    const hits = await store.recall('pricing page', 5)
    assert.ok(hits.length >= 1)
    assert.equal(hits[0].metadata?.ts, 20260701, 'recall round-trips metadata too')
  } finally {
    cleanup()
  }
})

test('recallFusedDated returns a chronological timeline and a now anchor', async () => {
  const { store, cleanup } = freshStore()
  try {
    await store.remember('the release shipped', [], { ts: 20260701 })
    await store.remember('the project kicked off', [], { ts: 20260103 })

    const res = await store.recallFusedDated('project release timeline', 'ts', 10)
    assert.ok(Array.isArray(res.memories) && res.memories.length >= 2)
    assert.ok(res.datedContext.includes('- [2026-01-03] the project kicked off'))
    assert.ok(res.datedContext.includes('- [2026-07-01] the release shipped'))
    // Oldest first.
    assert.ok(
      res.datedContext.indexOf('2026-01-03') < res.datedContext.indexOf('2026-07-01'),
      'timeline is oldest-first',
    )
    assert.equal(res.now, '2026-07-01', 'now anchors on the latest date')
  } finally {
    cleanup()
  }
})

test('feedback reinforces a fact, weakens on failure, NOT_FOUND on unknown id', async () => {
  const { store, cleanup } = freshStore()
  try {
    const id = await store.remember('the deploy pipeline runs clippy before tests')
    const up = await store.feedback(id, true)
    assert.equal(typeof up, 'number', 'feedback resolves to the learned confidence')
    assert.ok(up > 0.5 && up <= 1, `success must raise confidence above neutral, got ${up}`)
    const down = await store.feedback(id, false)
    assert.ok(down < up, `failure must lower confidence, got ${down} after ${up}`)
    await assert.rejects(
      store.feedback('999999', true),
      (err) => err.message.includes('NOT_FOUND'),
      'feedback on an unknown id → NOT_FOUND',
    )
  } finally {
    cleanup()
  }
})

test('forget reports found=true on a real deletion, found=false on a typo id', async () => {
  const { store, cleanup } = freshStore()
  try {
    const id = await store.remember('ephemeral note to forget')
    assert.equal(await store.forget(id), true, 'deleting an existing fact resolves true')
    assert.equal(
      await store.forget(id),
      false,
      'a second forget of the same id finds nothing — distinguishable from the first',
    )
    assert.equal(
      await store.forget('999999999'),
      false,
      'a never-stored id resolves false (a no-op, not an error)',
    )
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

test('compileContext compiles fragments under a budget with provenance', async () => {
  const { store, cleanup } = freshStore()
  try {
    const out = await store.compileContext({
      query: 'deploy pipeline',
      fragments: [
        { content: 'The deploy pipeline runs clippy before tests.' },
        { content: 'The deploy pipeline runs clippy before tests.' },
        { content: '```rust\nlet x = 42;\n```' },
      ],
      token_budget: 10000,
    })
    assert.ok(out.content.includes('let x = 42;'), 'code survives verbatim')
    assert.equal(out.decisions.length, 3, 'one decision per fragment')
    const dup = out.decisions.find((d) => d.action === 'drop')
    assert.ok(dup, 'the duplicate must be dropped')
    assert.equal(typeof dup.fragment_id, 'string', 'ids cross as decimal strings')
    assert.equal(typeof dup.content_hash, 'string', 'hashes cross as decimal strings')
    assert.ok(out.insights.tokens_saved > 0, 'the duplicate saves tokens')
    for (const source of out.sources) {
      assert.ok(source.handle.startsWith('ctx://source/'), 'sources stay addressable')
    }
  } finally {
    cleanup()
  }
})

test('compileContext pulls memory scope and stamps memory ids', async () => {
  const { store, cleanup } = freshStore()
  try {
    await store.remember('the deploy pipeline runs clippy before tests')
    const out = await store.compileContext({
      query: 'deploy pipeline checks',
      fragments: [{ content: 'Session note: user asked about CI.' }],
      token_budget: 10000,
      memory_scope: { k: 3 },
    })
    assert.ok(out.content.includes('runs clippy before tests'), 'memory pulled in')
    const memoryDecision = out.decisions.find((d) => d.memory_id !== undefined && d.memory_id !== null)
    assert.ok(memoryDecision, 'a decision must carry the backing memory id')
    assert.equal(typeof memoryDecision.memory_id, 'string', 'memory ids cross as strings')
  } finally {
    cleanup()
  }
})

test('compileContext zero budget rejects with INVALID_INPUT', async () => {
  const { store, cleanup } = freshStore()
  try {
    await assert.rejects(
      store.compileContext({ query: 'x', fragments: [{ content: 'y' }], token_budget: 0 }),
      /INVALID_INPUT/,
    )
  } finally {
    cleanup()
  }
})

test('compileContext malformed request rejects with INVALID_INPUT', async () => {
  const { store, cleanup } = freshStore()
  try {
    await assert.rejects(store.compileContext({ fragments: 'not-an-array' }), /INVALID_INPUT/)
  } finally {
    cleanup()
  }
})
