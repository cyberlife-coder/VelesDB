// Success-path coverage for rememberExtracted — the auto-extraction feature.
//
// Real extraction needs a local Ollama generative model, which offline CI does
// not have, so this is OPT-IN: set VELESDB_OLLAMA_TESTS=1 (and have `ollama serve`
// running with the model below) to exercise it. Without the env var it is skipped,
// so the default `node --test` stays offline. The failure path (no backend →
// throws) is covered unconditionally in index.spec.mjs.
//
//   VELESDB_OLLAMA_TESTS=1 node --test __test__/remember_extracted.integration.spec.mjs

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { MemoryService } from '../index.js'

const RUN = !!process.env.VELESDB_OLLAMA_TESTS
const MODEL = process.env.VELESDB_OLLAMA_EXTRACT_MODEL ?? 'qwen3.6:27b-mlx'

test(
  'rememberExtracted: a local model extracts facts and auto-wires a graph why() can walk',
  { skip: RUN ? false : 'set VELESDB_OLLAMA_TESTS=1 (and run ollama) to enable' },
  async () => {
    const dir = mkdtempSync(join(tmpdir(), 'velesdb-node-extract-'))
    const store = MemoryService.open(dir, 'ollama', null, 'all-minilm')
    try {
      const transcript =
        'Standup: we moved the analytics export to 3am because it kept colliding ' +
        'with the database backup, which caused the Sunday slowdown. The backup ' +
        'cannot move because the storage vendor only gives a midnight window.'

      // Success path: returns the stored fact ids — no relate()/links by hand.
      const ids = await store.rememberExtracted(transcript, MODEL)
      assert.ok(Array.isArray(ids), 'returns an array of ids')
      assert.ok(ids.length >= 2, `extracted multiple facts (got ${ids.length})`)
      for (const id of ids) assert.match(id, /^\d+$/, 'each id is a decimal string')

      // The auto-built graph is traversable: why() reaches a connected fact that
      // shares no words with the question's surface.
      const { nodes } = await store.why('why did the analytics export move to 3am?', 4)
      assert.ok(nodes.length >= 2, 'why() returns a connected subgraph')
      const reached = nodes.map((n) => n.content).join(' ').toLowerCase()
      assert.ok(
        reached.includes('backup') || reached.includes('vendor') || reached.includes('collid'),
        'why() walked the self-built graph to the root cause',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  },
)
