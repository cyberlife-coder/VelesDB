// Committed measurement harness for the context compiler (see ../README.md).
// Prereqs: from crates/velesdb-node run 'npm ci && npm run build' then
// 'npm install --no-save gpt-tokenizer'. Run this file with plain 'node'.
//
// TRI-ENGINE RESCUE BENCHMARK — measures what VelesDB's fused memory
// selection (HNSW vector seed + graph BFS reach + fusion ranking) adds to
// context compilation over a vector-only baseline, on the hardest realistic
// case: the evidence that answers the question shares FEW OR NO WORDS with
// it. Lexical/vector matching alone cannot surface such facts; the graph
// walk from the vector seed can — that is the tri-engine's job.
//
// Setup: a small incident knowledge base is stored the way the
// velesdb-memory skill prescribes (remember + relate: decision -> cause ->
// fix chains). Each benchmark case asks a question and compares, over the
// same store and the same k:
//   A. vector-only recall(query, k)          — what a plain vector DB does
//   B. compileContext with memory_scope      — HNSW seed + BFS hops + fusion,
//      compiled straight into a budgeted, provenance-carrying context
// Scoring is mechanical: which of the case's answer-bearing facts appear.
// Everything is deterministic (hash embedder, no clock) and token counts
// use a real cl100k BPE.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const nodeCrate = new URL('../../../../velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
const { encode } = require('gpt-tokenizer')
const { MemoryService } = require(nodeCrate + 'index.js')

const bpe = (s) => encode(s).length

// --- The incident knowledge base (stored per the velesdb-memory skill) -----
// Three cause/fix chains plus operational distractors. The deeper facts of
// each chain deliberately share no vocabulary with the questions asked
// about their surface symptom — the realistic post-mortem shape: symptoms
// are described in user language, root causes in infrastructure language.

const FACTS = {
  // Chain 1: checkout failures -> retry storm -> pool timeout fix
  checkout: 'Checkout requests fail with 502 errors whenever the payments service is overloaded during peak traffic.',
  retry_storm: 'Aggressive client retry storms exhaust the gateway connection pool within about ninety seconds.',
  pool_fix: 'Raising the pool acquisition timeout from five to forty-five seconds stopped the cascade for good.',
  // Chain 2: search latency -> compaction stalls -> io scheduler fix
  search_slow: 'Search latency spikes above two seconds every night around three in the morning.',
  compaction: 'Background segment compaction saturates disk bandwidth and stalls foreground reads.',
  ionice_fix: 'Capping compaction throughput at forty megabytes per second eliminated the nightly stalls.',
  // Chain 3: login errors -> clock skew -> ntp fix
  login_fail: 'Users intermittently see login errors saying their session token is invalid or expired.',
  clock_skew: 'Clock skew between the auth nodes exceeded the token validation tolerance window.',
  ntp_fix: 'Enabling chrony with a tighter polling interval brought all nodes within ten milliseconds.',
  // Distractors — plausible ops facts, unrelated to the questions
  d1: 'The weekly dependency audit runs cargo deny on every workspace crate.',
  d2: 'Grafana dashboards are provisioned from JSON files checked into the ops repository.',
  d3: 'The staging cluster is rebuilt from scratch every Sunday night by the infra pipeline.',
  d4: 'Deploy notifications are posted to the release channel with the changelog attached.',
}

// decision -> cause -> fix edges, pointing outward from the fact an agent
// will ask about (the direction why()/fused reach traverse).
const LINKS = [
  ['checkout', 'retry_storm', 'caused_by'],
  ['retry_storm', 'pool_fix', 'fixed_by'],
  ['search_slow', 'compaction', 'caused_by'],
  ['compaction', 'ionice_fix', 'fixed_by'],
  ['login_fail', 'clock_skew', 'caused_by'],
  ['clock_skew', 'ntp_fix', 'fixed_by'],
]

// --- Benchmark cases -------------------------------------------------------
// `expect` lists the answer-bearing facts; `overlap` marks which of them
// share vocabulary with the question (reachable by lexical/vector match)
// versus which only the graph can reach.

const CASES = [
  {
    q: 'why do checkout requests fail during peak traffic and what fixed it',
    expect: ['checkout', 'retry_storm', 'pool_fix'],
  },
  {
    q: 'what causes the nightly search latency spikes and what was the remedy',
    expect: ['search_slow', 'compaction', 'ionice_fix'],
  },
  {
    q: 'users report login errors about invalid session tokens, what is going on',
    expect: ['login_fail', 'clock_skew', 'ntp_fix'],
  },
]

const dir = mkdtempSync(join(tmpdir(), 'veles-tri-rescue-'))
const mem = MemoryService.open(dir, 'hash')

// Build the knowledge base.
const ids = {}
for (const [key, text] of Object.entries(FACTS)) ids[key] = await mem.remember(text)
for (const [from, to, rel] of LINKS) await mem.relate(ids[from], ids[to], rel)

const K = 5
const found = (texts, keys) => keys.filter((k) => texts.some((t) => t.includes(FACTS[k])))

console.log('TRI-ENGINE RESCUE BENCHMARK (HNSW seed + graph BFS + fusion vs vector-only)')
console.log(`knowledge base: ${Object.keys(FACTS).length} facts, ${LINKS.length} typed edges | k=${K} | hash embedder (lexical vector space) | real cl100k tokens`)
console.log('')

let totalExpected = 0
let vectorFound = 0
let fusedFound = 0
let rescued = 0
let compiledAnswerable = 0

for (const [i, c] of CASES.entries()) {
  // A. vector-only baseline — what a plain vector database returns.
  const t0 = process.hrtime.bigint()
  const plain = await mem.recall(c.q, K)
  const vecMs = Number(process.hrtime.bigint() - t0) / 1e6
  const vecHits = found(plain.map((r) => r.content), c.expect)

  // B. fused with crate defaults (hops=2, graph_boost=0.15 — the LoCoMo
  // conversational tuning).
  const compileWith = (scope) =>
    mem.compileContext({
      query: c.q,
      token_budget: 400,
      fragments: [{ content: 'You are the on-call assistant. Answer strictly from the provided context.', metadata: { cache: true } }],
      memory_scope: scope,
      policy: { store_sources: false, record_events: false },
    })
  const outDefault = await compileWith({ k: K })
  const defaultHits = found([outDefault.content], c.expect)

  // C. fused with the scope's graph knobs raised for curated fact chains —
  // exactly what the skill prescribes when memory holds relate-linked
  // cause/fix chains.
  const t1 = process.hrtime.bigint()
  const out = await compileWith({ k: K, graph_boost: 0.6 })
  const fusedMs = Number(process.hrtime.bigint() - t1) / 1e6
  const fusedHits = found([out.content], c.expect)
  const memoryBacked = out.decisions.filter((d) => d.memory_id != null).length
  const rescuedHere = fusedHits.filter((k) => !vecHits.includes(k))

  totalExpected += c.expect.length
  vectorFound += vecHits.length
  fusedFound += fusedHits.length
  rescued += rescuedHere.length
  const answerable = fusedHits.length === c.expect.length
  if (answerable) compiledAnswerable++

  console.log(`case ${i + 1}: "${c.q}"`)
  console.log(`  vector-only recall           : ${vecHits.length}/${c.expect.length} answer facts  (${vecMs.toFixed(1)} ms)  -> ${vecHits.join(', ') || 'none'}`)
  console.log(`  fused, default knobs         : ${defaultHits.length}/${c.expect.length} answer facts  -> ${defaultHits.join(', ') || 'none'}`)
  console.log(`  fused, graph_boost=0.6       : ${fusedHits.length}/${c.expect.length} answer facts  (${fusedMs.toFixed(1)} ms)  -> ${fusedHits.join(', ')}`)
  console.log(`  graph rescue vs vector-only  : ${rescuedHere.length} fact(s) only the BFS reach surfaced: ${rescuedHere.join(', ') || '-'}`)
  console.log(`  compiled context             : ${bpe(out.content)} real tokens, ${memoryBacked} memory-backed decisions with provenance, answerable=${answerable}`)
  console.log('')
}

console.log('--- totals ---')
console.log(`answer-fact coverage : vector-only ${vectorFound}/${totalExpected}  vs  fused ${fusedFound}/${totalExpected}`)
console.log(`graph rescues        : ${rescued}/${totalExpected} answer facts were reachable ONLY through the typed-edge walk`)
console.log(`fully answerable     : ${compiledAnswerable}/${CASES.length} compiled contexts contain every answer fact`)

// Determinism: same store, same calls, byte-identical compiled output.
const repro = (n) =>
  mem.compileContext({
    query: CASES[0].q,
    token_budget: 400,
    fragments: [{ content: 'You are the on-call assistant. Answer strictly from the provided context.', metadata: { cache: true } }],
    memory_scope: { k: K, graph_boost: 0.6 },
    policy: { store_sources: false, record_events: false },
  })
const again = await repro(1)
const once = await repro(2)
console.log(`reproducibility      : ${again.content === once.content ? 'OK (byte-identical)' : 'FAILED'}`)

// --- RL x GRAPH SYNERGY (EPIC-P-071/US-002 importance blend) ---------------
// One ranking out of every engine: a fact the team reinforced via feedback()
// AND that only the graph BFS reaches must out-rank a fact that is merely
// lexically similar to the question. Runs AFTER the measurements above on
// purpose — feedback() mutates the learned confidence recall() re-ranks by.
//
// Setup, per the real triage workflow: a wordy report fact that shares the
// question's vocabulary but answers nothing is added and flagged as noise;
// the zero-overlap pool fix (graph-reached through checkout -> retry_storm
// -> pool_fix) is repeatedly marked useful.
console.log('')
console.log('RL x GRAPH SYNERGY (importance blend over the fused pool)')
const SIMILAR_ONLY = 'Failed checkout attempts during peak periods are tallied in the weekly payments report.'
const similarId = await mem.remember(SIMILAR_ONLY)
for (let i = 0; i < 20; i++) await mem.feedback(ids.pool_fix, true)
for (let i = 0; i < 5; i++) await mem.feedback(similarId, false)

const synergyCompile = (importance) =>
  mem.compileContext({
    query: CASES[0].q,
    token_budget: 500,
    fragments: [{ content: 'You are the on-call assistant. Answer strictly from the provided context.', metadata: { cache: true } }],
    memory_scope: { k: K, graph_boost: 0.6 },
    policy: { store_sources: false, record_events: false, importance },
  })

const synergyRun = async () => {
  // Blend OFF (zero weights): the wordy near-miss out-ranks the real fix.
  const off = await synergyCompile({ confidence: 0.0, recency: 0.0 })
  // Blend ON: learned confidence re-ranks INSIDE the fused-selected pool.
  const on = await synergyCompile({ confidence: 0.8, recency: 0.0 })
  const posOf = (out, text) => out.content.indexOf(text)
  return {
    offFixPos: posOf(off, FACTS.pool_fix),
    offSimPos: posOf(off, SIMILAR_ONLY),
    onFixPos: posOf(on, FACTS.pool_fix),
    onSimPos: posOf(on, SIMILAR_ONLY),
    onContent: on.content,
    onReasons: on.decisions.filter((d) => d.memory_id != null).map((d) => d.reason.slice(d.reason.indexOf('pulled from memory'))),
  }
}

const r1 = await synergyRun()
const r2 = await synergyRun()
const present = (p) => p >= 0
const offLeads = present(r1.offSimPos) && present(r1.offFixPos) && r1.offSimPos < r1.offFixPos
const onFixLeads = present(r1.onFixPos) && present(r1.onSimPos) && r1.onFixPos < r1.onSimPos
console.log(`  blend OFF (weights 0)        : similar-only fact precedes the graph-reached fix: ${offLeads} (sim@${r1.offSimPos}, fix@${r1.offFixPos}) — 0.8.0 behaviour`)
console.log(`  blend ON  (confidence 0.8)   : reinforced graph-reached fix precedes the similar-only fact: ${onFixLeads} (fix@${r1.onFixPos}, sim@${r1.onSimPos})`)
for (const reason of r1.onReasons) console.log(`    ${reason}`)
const synergyReproducible = JSON.stringify(r1) === JSON.stringify(r2)
console.log(`  reproducibility              : ${synergyReproducible ? 'OK (two identical runs)' : 'FAILED'}`)

rmSync(dir, { recursive: true, force: true })
if (fusedFound <= vectorFound || again.content !== once.content) process.exit(1)
if (!offLeads || !onFixLeads || !synergyReproducible) process.exit(1)
