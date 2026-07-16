// Committed measurement harness for the context compiler (see ../README.md).
// Prereqs: from crates/velesdb-node run 'npm ci && npm run build' then
// 'npm install --no-save gpt-tokenizer'. Run this file with plain 'node'.
// Stress at the DoS caps through the optimized addon.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const nodeCrate = new URL('../../../../velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
const { MemoryService } = require(nodeCrate + 'index.js')

const dir = mkdtempSync(join(tmpdir(), 'veles-stress-'))
const mem = MemoryService.open(dir, 'hash')
const policy = { record_events: false, store_sources: false } // isolate pure compile cost

function frag(i, size) {
  let s = `Observation ${i}: worker retried batch after upstream drop near shard window ${i % 50}. `
  while (s.length < size) s += `Detail ${i}-${s.length}: latency spike observed, retry scheduled with backoff. `
  return { content: s.slice(0, size) }
}

async function run(label, fragments, budget) {
  const t0 = process.hrtime.bigint()
  const out = await mem.compileContext({ query: 'worker retries shard window', token_budget: budget, fragments, policy })
  const ms = Number(process.hrtime.bigint() - t0) / 1e6
  const bytes = fragments.reduce((a, f) => a + f.content.length, 0)
  console.log(`${label.padEnd(46)} | in ${(bytes / 1e6).toFixed(1)}MB | ${String(out.decisions.length).padStart(4)} decisions | risk ${out.risk.padEnd(6)} | ${ms.toFixed(0).padStart(6)} ms`)
  return out
}

console.log('STRESS AT THE CAPS (release addon, sources/events off)')
await run('1024 fragments x 1KB (1MB total), budget 4k', Array.from({length: 1024}, (_, i) => frag(i, 1024)), 4000)
await run('1024 fragments x 10KB (10MB total), budget 8k', Array.from({length: 1024}, (_, i) => frag(i, 10240)), 8000)
await run('64 fragments x 1MB (64MB total, cap size), b 8k', Array.from({length: 64}, (_, i) => frag(i, 1048576)), 8000)
await run('one 1MB repetitive log, budget 2k', [{ content: Array.from({length: 20000}, (_, i) => i % 3 === 0 ? 'ERROR timeout connecting to shard-3' : i % 3 === 1 ? 'WARN retrying upstream' : 'ERROR timeout connecting to shard-3').join('\n'), kind: 'log' }], 2000)
await run('1023 duplicates + 1 original, budget 4k', Array.from({length: 1024}, () => ({ content: 'the deploy pipeline runs clippy before tests and cargo deny after them' })), 4000)
console.log(`\npeak RSS: ${(process.memoryUsage().rss / 1e6).toFixed(0)} MB`)
rmSync(dir, { recursive: true, force: true })
