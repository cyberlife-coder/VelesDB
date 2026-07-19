// Committed measurement harness for the context compiler (see ../README.md).
// Prereqs: from crates/velesdb-node run 'npm ci && npm run build' then
// 'npm install --no-save gpt-tokenizer'. Run this file with plain 'node'.
//
// AGENT-SESSION BENCHMARK — the conditions an agent like Claude actually
// operates in when following the velesdb-context-optimizer skill: a
// deterministic 12-turn coding/debugging session whose context ACCUMULATES
// (system preamble, growing turn history, tool results including the same
// file read twice, a repetitive CI log, constraints, exact values). At each
// turn the naive agent would send the whole accumulated context raw; the
// skill-following agent calls compileContext (budget from argv, default
// 4000) and sends the compiled output instead. Everything is counted with a
// REAL cl100k BPE (gpt-tokenizer) — no estimates in the printed savings.
//
// Prints per-turn raw vs compiled real tokens and compile latency, then the
// session totals, the byte-stability of the cache-marked prefix across
// turns (prompt-cache material), and asserts token-figure reproducibility
// by compiling every turn twice.
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createRequire } from 'node:module'
const nodeCrate = new URL('../../../../velesdb-node/', import.meta.url).pathname
const require = createRequire(nodeCrate)
const { encode } = require('gpt-tokenizer')
const { MemoryService } = require(nodeCrate + 'index.js')

const bpe = (s) => encode(s).length
const budget = Number(process.argv[2] ?? 4000)

// --- Deterministic session fixtures (a realistic agent debugging session) ---

const SYSTEM = {
  content:
    'You are the coding agent for the veles repository. Follow the house rules: ' +
    'run the full gate suite before any commit, keep functions under fifty lines, ' +
    'never push directly to main, and answer from the provided context only.',
  metadata: { cache: true },
}

// The same source file, read by a tool at turn 2 and re-read at turn 7 —
// the classic duplicated-tool-result waste in real agent transcripts.
const SOURCE_FILE = {
  content:
    '```rust\n' +
    'pub fn rebalance(cluster: &mut Cluster, plan: &Plan) -> Result<Report, Error> {\n' +
    '    let mut moved = 0_u64;\n' +
    '    for step in plan.steps() {\n' +
    '        let shard = cluster.shard_mut(step.shard_id).ok_or(Error::UnknownShard)?;\n' +
    '        shard.pause_ingestion();\n' +
    '        let bytes = shard.transfer_to(step.target_node)?;\n' +
    '        moved = moved.saturating_add(bytes);\n' +
    '        shard.resume_ingestion();\n' +
    '        telemetry::record_move(step.shard_id, step.target_node, bytes);\n' +
    '    }\n' +
    '    Ok(Report { moved, steps: plan.steps().len() })\n' +
    '}\n' +
    '```',
  kind: 'code',
}

// A CI log tool result: 180 lines, 4 distinct messages (kind: log so the
// compiler may collapse repeats with counts).
const ciLog = () => {
  const lines = []
  for (let i = 0; i < 180; i++) {
    lines.push(
      i % 45 === 0
        ? 'INFO  gate suite started on runner linux-x64-large'
        : i % 45 === 1
          ? 'WARN  retrying flaky network fetch for crates.io index'
          : i % 45 === 2
            ? 'ERROR test rebalance_pauses_ingestion timed out after 60s'
            : 'ERROR timeout connecting to shard-3 during rebalance fixture',
    )
  }
  return { content: lines.join('\n'), kind: 'log' }
}

const CONSTRAINT = {
  content: 'Never resume ingestion on a shard whose transfer returned an error.',
}
const EXACT_VALUES = {
  content:
    'Failing build 7f3a-11 on commit bd97d3cd moved 1_048_576 bytes across 12 shards before timing out at 60s.',
}

// What each turn ADDS to the accumulated context (user msg + agent answer of
// the previous turn + occasional tool results). Deterministic, no clock.
const TURN_EVENTS = [
  [{ content: 'User: the rebalance test times out in CI since this morning, can you investigate?' }],
  [{ content: 'Agent (turn 1): I will start by reading the rebalance implementation and the CI log.' }, SOURCE_FILE],
  [{ content: 'Agent (turn 2): the transfer loop pauses ingestion per shard; I need the CI log to see where it hangs.' }, ciLog()],
  [{ content: 'Agent (turn 3): the log shows repeated shard-3 connection timeouts during the rebalance fixture.' }, EXACT_VALUES],
  [{ content: 'User: careful, production had an incident last week related to ingestion.' }, CONSTRAINT],
  [{ content: 'Agent (turn 5): acknowledged the constraint; checking whether the fixture resumes ingestion after a failed transfer.' }],
  [{ content: 'Agent (turn 6): re-reading the source to verify the error path around transfer_to.' }, SOURCE_FILE],
  [{ content: 'Agent (turn 7): confirmed - resume_ingestion runs even when transfer_to fails, matching the incident pattern.' }],
  [{ content: 'User: so the test hangs because shard-3 never recovers? propose a fix.' }],
  [{ content: 'Agent (turn 9): proposing to move resume_ingestion into a drop guard so the error path cannot skip telemetry or resume.' }],
  [{ content: 'Agent (turn 10): drafting the patch and the regression test test_rebalance_error_path_never_resumes_silently.' }],
  [{ content: 'User: run the gates and summarize what changed before committing.' }],
]

// --- The session -------------------------------------------------------------

const dir = mkdtempSync(join(tmpdir(), 'veles-agent-session-'))
const mem = MemoryService.open(dir, 'hash')

console.log('AGENT-SESSION BENCHMARK (real cl100k tokens, gpt-tokenizer)')
console.log(`12 accumulating turns, compileContext budget ${budget}, node addon`)
console.log('')
console.log('turn | fragments | raw_tokens | compiled_tokens | saved% | risk | latency_ms')

const accumulated = [SYSTEM]
let totalRaw = 0
let totalCompiled = 0
let latencies = []
let cachePrefixes = new Set()
let reproducible = true

for (let turn = 0; turn < TURN_EVENTS.length; turn++) {
  accumulated.push(...TURN_EVENTS[turn])
  const rawTokens = bpe(accumulated.map((f) => f.content).join('\n\n'))

  const req = {
    query: 'why does the rebalance test time out and how do we fix it safely',
    token_budget: budget,
    fragments: accumulated,
  }
  const t0 = process.hrtime.bigint()
  const out = await mem.compileContext(req)
  const ms = Number(process.hrtime.bigint() - t0) / 1e6
  const again = await mem.compileContext(req)
  if (out.content !== again.content) reproducible = false

  const compiledTokens = bpe(out.content)
  totalRaw += rawTokens
  totalCompiled += compiledTokens
  latencies.push(ms)
  const cache = (out.sections ?? []).find((s) => s.kind === 'cache')
  if (cache) cachePrefixes.add(cache.content)

  const saved = ((1 - compiledTokens / rawTokens) * 100).toFixed(1)
  console.log(
    `${String(turn + 1).padStart(4)} | ${String(accumulated.length).padStart(9)} | ${String(rawTokens).padStart(10)} | ${String(compiledTokens).padStart(15)} | ${saved.padStart(5)}% | ${out.risk.padEnd(4)} | ${ms.toFixed(1).padStart(8)}`,
  )
}

// Second pass: the same session compiled STATELESSLY (no source/event
// persistence) — isolates the pure compile cost from the provenance
// persistence an operator can opt out of.
const statelessLat = []
{
  const acc = [SYSTEM]
  for (let turn = 0; turn < TURN_EVENTS.length; turn++) {
    acc.push(...TURN_EVENTS[turn])
    const t0 = process.hrtime.bigint()
    await mem.compileContext({
      query: 'why does the rebalance test time out and how do we fix it safely',
      token_budget: budget,
      fragments: acc,
      policy: { store_sources: false, record_events: false },
    })
    statelessLat.push(Number(process.hrtime.bigint() - t0) / 1e6)
  }
}
const statelessMean = (statelessLat.reduce((a, b) => a + b, 0) / statelessLat.length).toFixed(1)
const statelessMax = Math.max(...statelessLat).toFixed(1)

const sessionSaved = ((1 - totalCompiled / totalRaw) * 100).toFixed(1)
const meanMs = (latencies.reduce((a, b) => a + b, 0) / latencies.length).toFixed(1)
const maxMs = Math.max(...latencies).toFixed(1)

console.log('')
console.log(`session totals: ${totalRaw} raw -> ${totalCompiled} compiled real tokens = ${sessionSaved}% saved`)
console.log(`compile latency (with source/event persistence, default): mean ${meanMs} ms, max ${maxMs} ms`)
console.log(`compile latency (stateless: store_sources/record_events off): mean ${statelessMean} ms, max ${statelessMax} ms`)
console.log(
  `cache-marked prefix: ${cachePrefixes.size === 1 ? `byte-stable across all ${TURN_EVENTS.length} turns (${bpe([...cachePrefixes][0])} real tokens reusable by provider prompt caching)` : `NOT stable (${cachePrefixes.size} variants)`}`,
)
console.log(`reproducibility: ${reproducible ? 'OK (every turn compiled twice, byte-identical)' : 'FAILED'}`)

rmSync(dir, { recursive: true, force: true })
if (!reproducible || cachePrefixes.size !== 1) process.exit(1)
