// Committed measurement harness for the context compiler (see ../README.md).
// Prereqs: from crates/velesdb-node run 'npm ci && npm run build' then
// 'npm install --no-save gpt-tokenizer'. Run this file with plain 'node'.
// Real-tokenizer measurement: compile the benchmark-style corpus through the
// built addon and count tokens with a REAL BPE (cl100k via gpt-tokenizer).
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const nodeCrate = new URL('../../../../velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
// gpt-tokenizer resolves from the node crate too (installed --no-save there).
const { encode } = require('gpt-tokenizer')
const { MemoryService } = require(nodeCrate + 'index.js')

const bpe = (s) => encode(s).length

// Rebuild the committed benchmark corpus (same shape as examples/context_savings).
const fragments = []
fragments.push({ content: 'You are the deploy assistant for the veles cluster. Answer from the provided context only.', metadata: { cache: true } })
for (let turn = 0; turn < 8; turn++) {
  fragments.push({ content: `Turn ${turn}: the user asked about the state of the deploy pipeline and whether the canary stage passed its checks before promotion.` })
  fragments.push({ content: 'The deploy pipeline runs clippy, the test suite, and cargo deny before any artifact is promoted to the canary stage.' })
}
fragments.push({ content: '```rust\nfn promote(candidate: Build) -> Result<(), DeployError> {\n    candidate.verify_checksums()?;\n    canary::roll(candidate, Percent(5))\n}\n```', kind: 'code' })
fragments.push({ content: 'Never restart the primary node during a rebalance.' })
fragments.push({ content: 'Rollout 7f3a promoted 2026-07-14 with 1_048_576 bytes shipped across 12 shards.' })
const logLines = []
for (let i = 0; i < 120; i++) logLines.push(i % 40 === 0 ? 'INFO canary check passed for shard-1' : i % 40 === 1 ? 'WARN retrying upstream connection' : 'ERROR timeout connecting to shard-3')
fragments.push({ content: logLines.join('\n'), kind: 'log' })

const rawText = fragments.map((f) => f.content).join('\n\n')
const rawBpe = bpe(rawText)

const dir = mkdtempSync(join(tmpdir(), 'veles-tokmeasure-'))
const mem = MemoryService.open(dir, 'hash')

console.log('REAL-TOKENIZER MEASUREMENT (cl100k BPE, gpt-tokenizer)')
console.log(`raw context: ${rawBpe} real tokens (${rawText.length} chars)`)
console.log('')
console.log('budget | est_out | real_out | real saved% | est/real error | fits_budget(real)')
for (const budget of [400, 800, 1600, 3200]) {
  const out = await mem.compileContext({
    query: 'state of the deploy pipeline and canary checks',
    token_budget: budget,
    fragments,
  })
  const realOut = bpe(out.content)
  const estOut = out.insights.tokens_out
  const savedPct = ((rawBpe - realOut) * 100 / rawBpe).toFixed(1)
  const errPct = ((estOut - realOut) * 100 / Math.max(realOut, 1)).toFixed(1)
  console.log(`${String(budget).padStart(6)} | ${String(estOut).padStart(7)} | ${String(realOut).padStart(8)} | ${String(savedPct).padStart(10)}% | ${String(errPct).padStart(13)}% | ${realOut <= budget}`)
}

// Determinism across two calls through the whole Node stack:
const a = await mem.compileContext({ query: 'q', token_budget: 800, fragments })
const b = await mem.compileContext({ query: 'q', token_budget: 800, fragments })
console.log('\nnode-stack determinism:', JSON.stringify(a) === JSON.stringify(b) ? 'OK (byte-identical)' : 'FAILED')
rmSync(dir, { recursive: true, force: true })
