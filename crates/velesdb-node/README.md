# @wiscale/velesdb-memory-node

**The explainable, local-first memory engine for AI agents — as an in-process
Node.js addon (napi-rs).** Same hardened Rust as the MCP server and the Python
binding; no network service. Under the hood it fuses vector + graph + columnar,
which is *how* it remembers, connects, and explains.

`remember` / `recall` / `recallWhere` / `recallFused` / `recallFusedDated` /
`relate` / `forget` / `why` / `feedback` / `rememberExtracted` /
`compileContext` / `compileTranscript` / `contextSavings` /
`explainCompilation` / `retrieveContextSource` / working contexts
(`save`/`load`/`list`). The differentiator is **`why()`**: it
answers a question with the best-matching memory *plus its connected subgraph*
— related facts a plain vector recall is blind to. `compileContext` applies
the same explainability to your token bill: deterministic context compression
with an auditable decision per fragment.

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](https://raw.githubusercontent.com/cyberlife-coder/VelesDB/develop/examples/agent_memory/why_across_sessions.gif)

> The store is on disk, so memory survives process restarts — a new session
> reopens it and `why()` still walks the graph to context that shares no words
> with the question.

## What you actually gain

Two problems cost you real money and real quality every day, and this addon
fixes both — in your own Node process, with no service to run.

**Your agent forgets.** Close the process and everything it learned is gone.
The store is a directory on disk, so a new process reopens it and the facts are
still there.

**Every turn re-sends the whole conversation.** That is what you are billed for,
and a context padded with repeated logs is also one where the model attends less
to what matters. `compileContext` shrinks that payload before you send it —
deterministically, and without calling any model itself.

| What improves | Measured | How it was measured |
|---|---|---|
| Context sent to the model | **82.5 % smaller** over a 12-turn coding session (80.8–87.4 % per turn as it grows) | [committed corpus, real cl100k tokenizer](../velesdb-memory/examples/context_savings) — every turn compiled twice, byte-identical |
| Compile cost | **0.7 ms** stateless, 24.5 ms with source/event persistence on | same run |
| Storing a memory | **zero AI calls** — nothing leaves the process | the write path never calls a model |
| Prompt-cache prefix | **byte-stable across all 12 turns** (45 tokens reusable) | same run |

Those percentages come from *our* corpus. Every figure is pinned to its
committed source by a [contract the CI enforces](../../docs/reference/promise-contract.json):
if one drifts from what the code produces, the build goes red.

## How it works, in four steps

Everything below runs in-process. No server, no network, no API key.

**1. It stores facts, not transcripts.** `remember` takes one fact — *"the API
port is 6333 because 3000 collided with the web UI"* — and writes it to a local
directory. No model call.

**2. It finds them by meaning.** `recall` matches on sense, so *"which port did
we settle on"* reaches that fact although the words differ.

**3. It connects them — the part a search engine cannot do.** Facts are linked
to the topics they mention, and `why` walks those links: it returns the best
match **plus the facts that explain it**, including ones sharing no words with
your question. That is what the GIF above shows across a restart.

> Those links have to exist. If you only ever call `remember`, the graph stays
> flat and `why` degrades to a search. `rememberExtracted` takes a paragraph,
> splits it into facts and wires the links for you.

**4. It compresses what is too big.** `compileContext` takes your accumulated
context and a token budget, and returns a compiled view with **one auditable
decision per fragment** — kept, abstracted, or dropped — plus a handle to fetch
any original back. Nothing is destroyed; `retrieveContextSource` returns the
exact bytes.

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

Wiring the API gives your agent the *methods*; it doesn't tell it *when* to use
them — the bundled
[`velesdb-memory` skill](./skills/velesdb-memory/SKILL.md) teaches the loop
(recall before acting → remember decisions with metadata **and** links →
`relate` facts as relationships appear → `why` to explain → `feedback` to
reinforce). Install it the same way as the context-optimizer skill below:

```bash
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-memory ~/.claude/skills/
```

**Keep it fresh:** this `cp` is a snapshot, not a live link — re-run it after
every `npm update` so the installed skill picks up doc/behavior changes from
the new package version (safe to repeat: it just overwrites the local copy).

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
teaches an agent the full workflow, including when *not* to compress. The
binding exposes the compiler's read tools natively too: `explainCompilation`
and `contextSavings` (alongside `compileContext`, `compileTranscript`,
`retrieveContextSource`, save/load/list working context, and `feedback`).
Install the skill into your agent's skills directory:

```bash
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-context-optimizer ~/.claude/skills/
```

**Keep it fresh:** re-run this after every `npm update` for the same reason
as the `velesdb-memory` skill above.

#### Media fragments (`retrieveContextSource`)

A fragment may carry an inline screenshot alongside its caption — set
`media: {mime, bytes_b64}` on a fragment:

```js
const out = await mem.compileContext({
  query: 'a screenshot of the failing build',
  token_budget: 4000,
  fragments: [
    { content: 'the failing build, before the fix', media: { mime: 'image/png', bytes_b64: pngB64 } },
  ],
})
```

The image packs atomically (never chunked) and costs tokens from its actual
pixels, not its base64 text — see the crate README's "Media fragments"
section for the full model. `out.sources[i].handle` is a pointer only
(`fragment_id` + `handle`); fetch the image itself back — inline or
externalized by budget, it makes no difference — with `retrieveContextSource`:

```js
const source = await mem.retrieveContextSource(out.sources[0].handle)
source.content   // the caption, byte for byte
source.media     // { mime, bytes_b64 } when the fragment carried one, else undefined
```

Same JSON shape as the MCP `retrieve_context_source` tool
(`{handle, content, media?}`); an unknown or expired handle rejects with
`NOT_FOUND`.

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
