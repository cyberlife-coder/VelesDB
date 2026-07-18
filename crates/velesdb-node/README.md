# @wiscale/velesdb-memory-node

**The explainable, local-first memory engine for AI agents — as an in-process
Node.js addon (napi-rs).** Same hardened Rust as the MCP server and the Python
binding; no network service. Under the hood it fuses vector + graph + columnar,
which is *how* it remembers, connects, and explains.

`remember` / `recall` / `recallWhere` / `relate` / `forget` / `why` /
`rememberExtracted` / `compileContext`. The differentiator is **`why()`**: it
answers a question with the best-matching memory *plus its connected subgraph*
— related facts a plain vector recall is blind to. `compileContext` applies
the same explainability to your token bill: deterministic context compression
with an auditable decision per fragment.

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](https://raw.githubusercontent.com/cyberlife-coder/VelesDB/develop/examples/agent_memory/why_across_sessions.gif)

> The store is on disk, so memory survives process restarts — a new session
> reopens it and `why()` still walks the graph to context that shares no words
> with the question.

## Install

```bash
npm install @wiscale/velesdb-memory-node
```

Prebuilt binaries ship for macOS (arm64/x64), Linux (x64/arm64 gnu), and Windows
(x64). Node >= 18.17.

## Usage

```js
import { MemoryService } from '@wiscale/velesdb-memory-node'

// Offline "hash" embedder by default; pass "ollama" for real semantic recall.
const mem = MemoryService.open('./agent_mem')

const pr = await mem.remember('PR #42 swaps the mutex for parking_lot')
const decision = await mem.remember(
  'we chose parking_lot to avoid lock poisoning',
  [{ target: pr, relation: 'decided_in' }],
)

// recall: vector similarity.
const hits = await mem.recall('lock poisoning', 5)

// recallWhere: fused vector + structured filters (ranges/comparisons).
const recent = await mem.recallWhere('release notes', [
  { field: 'ts', op: 'ge', value: 20260101 },
])

// why: the wedge — seed memory + its reachable subgraph.
const { nodes, edges } = await mem.why('why parking_lot')
```

Every method returns a `Promise` and runs off the event-loop thread. Memory ids
cross the boundary as **decimal strings** (a JS `number` loses precision above
2^53). Errors are `Error`s whose message is prefixed with a stable code:
`[INVALID_INPUT]`, `[NOT_FOUND]`, or `[INTERNAL]`.

### Auto-extraction (`rememberExtracted`)

```js
// Extract atomic facts from raw text with a local Ollama model and auto-build
// the fact↔topic graph that powers why().
const ids = await mem.rememberExtracted(longText, 'qwen3', 'http://localhost:11434')
```

### Context compilation (`compileContext`)

Your agent burns most of its tokens re-reading redundant context.
`compileContext` compresses it **deterministically** (no LLM, no cloud): the
same request always compiles to the same bytes, duplicates drop, repeated log
lines collapse with counts, code / URLs / numbers / negative constraints
survive verbatim, and over-budget content becomes a recoverable
`ctx://source/` handle — never a silent loss.

```js
const out = await mem.compileContext({
  query: 'state of the canary deploy',
  token_budget: 4000,
  memory_scope: { k: 5 }, // optional: pull relevant stored memories in
  fragments: [
    { content: 'You are the deploy assistant.', metadata: { cache: true } },
    { content: ciLogs, kind: 'log' },
    { content: 'Never restart the primary during a rebalance.' },
  ],
})

out.content            // the compiled prompt context (fits the budget)
out.risk               // 'low' | 'medium' | 'high' — 'high' means critical content did not fit
out.decisions          // one auditable decision per fragment (rule_id, reason, risk)
out.insights           // { tokens_in, tokens_out, tokens_saved, ... } — local estimates
```

The request/result JSON matches the MCP `compile_context` tool, with two
binding-wide differences: id fields (`fragment_id`, `content_hash`,
`memory_id`, `fragment_ids`, input `fragments[].id`) cross as decimal
strings, and the *top-level* result keys follow the binding's camelCase
(`out.retrievalHandles` — nested trees keep the wire's snake_case). `tokens_saved` is a local estimate, not billed tokens. The bundled
[`velesdb-context-optimizer` skill](./skills/velesdb-context-optimizer/SKILL.md)
teaches an agent the full workflow, including when *not* to compress. Install
it into your agent's skills directory:

```bash
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-context-optimizer ~/.claude/skills/
```

## Need the full engine?

This addon is the **memory wedge**: `remember` / `recall` / `relate` /
`forget` / `why` / `compileContext` — memory semantics only, by design (see
[License](#license) below). It does not expose raw VelesQL, deep graph
`MATCH`, collection administration, or any other database-shaped capability —
that would cross the
[`VelesDB Core License 1.0`](https://github.com/cyberlife-coder/VelesDB/blob/develop/LICENSE)
§1 "Substantial Set" line.

For the full engine (VelesQL, multi-hop `MATCH`, collection/index
administration) from Node/TypeScript, run the REST server and talk to it with
[`@wiscale/velesdb-sdk`](https://www.npmjs.com/package/@wiscale/velesdb-sdk)
instead:

```bash
# 1. Start the server (from source, or `cargo install velesdb-server`)
velesdb-server --port 8080
```

```typescript
// 2. Point the TypeScript SDK's REST backend at it.
import { VelesDB } from '@wiscale/velesdb-sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();

await db.createCollection('docs', { dimension: 4, metric: 'cosine' });
await db.upsert('docs', { id: 1, vector: [0.1, 0.2, 0.3, 0.4], payload: { title: 'Hello' } });

// Raw VelesQL — not available through this wedge.
const result = await db.query(
  'docs',
  'SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 5',
  { v: [0.1, 0.2, 0.3, 0.4] },
);
```

See the
[server README](https://github.com/cyberlife-coder/VelesDB/blob/develop/crates/velesdb-server/README.md)
for the full REST API (VelesQL, graph `MATCH`, auth, TLS) and the
[TypeScript SDK README](https://github.com/cyberlife-coder/VelesDB/blob/develop/sdks/typescript/README.md)
for the REST-backend client surface.

## License

VelesDB Core License 1.0 (based on ELv2). See [LICENSE](./LICENSE). This addon
exposes memory semantics only; it is not a hosted or managed service.
