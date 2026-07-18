// Committed measurement harness for the context compiler (see ../README.md).
// Prereqs: from crates/velesdb-node run 'npm ci && npm run build' then
// 'npm install --no-save gpt-tokenizer'. Run this file with plain 'node'.
//
// CACHE-PREFIX STABILITY — `compile_context` always emits the
// `Cache`-classified section first (see `sections()` in context.rs:
// `for kind in [SectionKind::Cache, SectionKind::Body]`), so a caller who
// marks its stable system/instructions fragments `metadata: {cache: true}`
// gets a byte-stable prefix across compiles, which is what lets a provider's
// prompt cache hit turn after turn. This script COMPILES 10 consecutive
// turns where the cache-marked fragments never change but the volatile
// content (a growing/changing log) does every turn, then measures the
// longest common byte prefix between every pair of consecutive outputs as a
// percentage of the cache section's own length — the number reported is
// exactly what was measured, nothing extrapolated.
//
// Cost: the compiler never hardcodes a rate (see PricingTable in
// insights.rs) — this script injects an EXAMPLE pricing table via
// `policy.pricing` (not a real provider's published rate; swap in your
// own) and reports, at that injected rate, the naive full-input-rate cost
// of the measured stable-prefix tokens if they were NOT cached and had to
// be resent as fresh input on every one of the 9 follow-up turns. That is
// an upper bound on the cache-read saving, not a promise of a specific
// discount — providers price a cache *read* well below a fresh input
// token, but that discount ratio is provider-specific and not modeled here.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const nodeCrate = new URL('../../../../velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
const { encode } = require('gpt-tokenizer')
const { MemoryService } = require(nodeCrate + 'index.js')

const bpe = (s) => encode(s).length
const TURNS = 10
const MODEL = 'example-model'

// EXAMPLE pricing table — caller-supplied, not a real provider rate.
const pricing = {
  version: 'example-2026-07-18',
  currency: 'USD',
  models: { [MODEL]: { input_micros_per_million_tokens: 3_000_000 } }, // $3 / 1M tokens, illustrative
}

const SYSTEM = {
  content: 'You are the deploy assistant for the veles cluster. Answer from the provided context only, and never restart the primary node during a rebalance.',
  metadata: { cache: true },
}
const RUNBOOK = {
  content: '```text\nRollback runbook: kubectl rollout undo deployment/canary; verify shard health before re-promoting.\n```',
  kind: 'code',
  metadata: { cache: true },
}

function volatileTurn(turn) {
  const lines = []
  for (let i = 0; i < 20; i++) {
    lines.push(`turn-${turn} shard-${i % 4}: ${i % 5 === 0 ? 'WARN retrying upstream connection' : 'INFO canary check passed'} at step ${turn * 20 + i}`)
  }
  return { content: lines.join('\n'), kind: 'log' }
}

const dir = mkdtempSync(join(tmpdir(), 'veles-cache-prefix-'))
const mem = MemoryService.open(dir, 'hash')
const policy = { record_events: false, store_sources: false, pricing }

const outputs = []
for (let turn = 0; turn < TURNS; turn++) {
  const out = await mem.compileContext({
    query: 'state of the deploy pipeline and canary checks',
    token_budget: 4000,
    target_model: MODEL,
    fragments: [SYSTEM, RUNBOOK, volatileTurn(turn)],
    policy,
  })
  outputs.push(out)
}

function commonPrefixLen(a, b) {
  const n = Math.min(a.length, b.length)
  let i = 0
  while (i < n && a[i] === b[i]) i++
  return i
}

console.log('CACHE-PREFIX STABILITY (10 consecutive compiles, volatile content changes every turn)')
console.log('')
const cacheSection = outputs[0].sections.find((s) => s.kind === 'cache')
if (!cacheSection) throw new Error('expected a cache section in turn 0 output')
const cacheLen = cacheSection.content.length
console.log(`cache section length: ${cacheLen} bytes (${bpe(cacheSection.content)} real tokens)`)
console.log('')
console.log('turn pair | common prefix bytes | % of cache section | prefix == full cache section')
let allFullyStable = true
for (let t = 1; t < TURNS; t++) {
  const prefixLen = commonPrefixLen(outputs[t - 1].content, outputs[t].content)
  const pct = ((Math.min(prefixLen, cacheLen) * 100) / cacheLen).toFixed(1)
  const fullyStable = prefixLen >= cacheLen
  allFullyStable &&= fullyStable
  console.log(`${String(t - 1).padStart(2)}->${String(t).padEnd(2)}    | ${String(prefixLen).padStart(20)} | ${String(pct).padStart(17)}% | ${fullyStable}`)
}
console.log(`\nall ${TURNS - 1} consecutive pairs fully cover the cache section: ${allFullyStable}`)

// Cost framing at the injected example rate.
const prefixTokens = bpe(cacheSection.content)
const microsPerToken = pricing.models[MODEL].input_micros_per_million_tokens / 1_000_000
const naiveResendCostMicros = Math.round(prefixTokens * microsPerToken * (TURNS - 1))
console.log(`\nat the EXAMPLE rate ($${pricing.models[MODEL].input_micros_per_million_tokens / 1e6}/1M input tokens), resending the ${prefixTokens}-token`)
console.log(`stable prefix as fresh input on all ${TURNS - 1} follow-up turns would cost ~${naiveResendCostMicros} micros`)
console.log('(this is an UPPER BOUND avoided by caching, not a promised discount — a real cache-read rate is provider-specific and lower than a fresh-input token, but that ratio is not modeled here).')

// Determinism: run the exact same 10-turn sequence again, compare digests.
const rerun = []
for (let turn = 0; turn < TURNS; turn++) {
  rerun.push(await mem.compileContext({
    query: 'state of the deploy pipeline and canary checks',
    token_budget: 4000,
    target_model: MODEL,
    fragments: [SYSTEM, RUNBOOK, volatileTurn(turn)],
    policy,
  }))
}
const identical = outputs.every((out, i) => out.content === rerun[i].content)
console.log(`\ndeterminism (same 10-turn sequence, two full runs): ${identical ? 'OK (byte-identical every turn)' : 'FAILED'}`)

rmSync(dir, { recursive: true, force: true })
