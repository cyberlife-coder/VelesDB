// Drop velesdb-memory into a Node agent in a few lines — same engine, JS runtime.
//
//   npm install @wiscale/velesdb-memory-node
//   node examples/why_magic_constant.mjs
//
// A coding agent recorded a decision and the human reason behind it. A later run
// reopens the on-disk memory. Plain recall surfaces the timeout line and look-alike
// config but is blind to *why* the value is 7. why() walks the typed link to the
// reason — so the agent warns you before you "round it down" and break a customer.
// Offline 'hash' embedder: no model, no network. (Verified embedder-robust — see README.)

import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MemoryService } from '../index.js'

const PROJECT_FACTS = [
  'The HTTP server listens on port 8080 by default',
  'We use clap for CLI argument parsing',
  'Benchmarks run nightly on the self-hosted M2 runner',
  'Auth tokens expire after 24 hours',
  'The release workflow publishes to crates.io on tag push',
  'Docs are built with mdBook and deployed to GitHub Pages',
  'We pin the Rust toolchain to 1.89 in CI',
  'The vector index uses HNSW with M=16',
  'Structured logging goes through tracing with JSON output in prod',
  'ColumnStore filters are pushed down before the NEAR scan',
  'We squash-merge every pull request into develop',
  'The WASM build drops the tokio dependency',
]

const store = MemoryService.open(mkdtempSync(join(tmpdir(), 'velesdb-demo-')), 'hash')

// An earlier session recorded the decision, its reason, and routine config.
const reason = await store.remember(
  "Our biggest customer's field crews work from remote mining sites over satellite links",
)
await store.remember('We set the default HTTP request timeout to 7 seconds', [
  { target: reason, relation: 'because' },
])
for (const fact of PROJECT_FACTS) await store.remember(fact)

const question = 'why is the request timeout set to 7 seconds?'

console.log(`\n  recall(${JSON.stringify(question)})   — vector similarity, top 5 of 14`)
for (const hit of await store.recall(question, 5)) {
  console.log(`     ${hit.score.toFixed(2)}  ${hit.content}`)
}
console.log('     └─ the reason is nowhere: it shares no words with the code.\n')

console.log(`  why(${JSON.stringify(question)})      — vector seed + graph of typed links`)
const { nodes } = await store.why(question, 2)
for (const node of nodes) console.log(`     hop ${node.hop}  ${node.content}`)
console.log("     └─ why() reached the real reason. Don't round 7 down — you'd")
console.log('        cut off the customer on the satellite link.')
