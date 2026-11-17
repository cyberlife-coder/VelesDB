# Node.js addon — `@wiscale/velesdb-memory-node`

> The complete surface of the in-process Node binding (`crates/velesdb-node`):
> every method, the JS-side contracts, the context compiler, the bundled agent
> skills, and the escape hatch when the memory wedge is not enough.

Start at the [crate README](../../crates/velesdb-node/README.md) for install
and a 60-second first success. This guide is the reference behind it.

## Contents

- [The 23-method surface](#the-23-method-surface)
- [Three contracts that apply to every method](#three-contracts-that-apply-to-every-method)
- [Choosing an embedder](#choosing-an-embedder)
- [Auto-extraction (`rememberExtracted`)](#auto-extraction-rememberextracted)
- [File anchors (`remember`/`recall`)](#file-anchors-rememberrecall)
- [Context compilation (`compileContext`)](#context-compilation-compilecontext)
- [Media fragments and `retrieveContextSource`](#media-fragments-and-retrievecontextsource)
- [Transcripts and working contexts](#transcripts-and-working-contexts)
- [Bundled agent skills](#bundled-agent-skills)
- [Need the full engine?](#need-the-full-engine)
- [Resource caps](#resource-caps)

## The 23-method surface

`MemoryService.open(path, embedder?, ollamaUrl?, ollamaModel?, searchQuality?)`
is the only constructor (a static factory). Everything else is an instance
method, and every instance method returns a `Promise`.

| Family | Methods |
|---|---|
| Durable memory | `remember`, `recall`, `recallWhere`, `recallFused`, `recallFusedDated`, `relate`, `unrelate`, `forget`, `entity`, `why`, `feedback`, `rememberExtracted` |
| Context compiler | `compileContext`, `compileTranscript`, `explainCompilation`, `contextSavings`, `retrieveContextSource`, `suggestBudget` |
| Session resumption | `saveWorkingContext`, `loadWorkingContext`, `listWorkingContexts` |
| Introspection | `memoryStatus`, `listMemories` |

That list is pinned by a test — `__test__/index.spec.mjs` asserts the exact
prototype allowlist and asserts that `query`, `upsert`, `createCollection`
and `traverse` are *absent*, so no raw-engine operation can slip in.

Parameter-by-parameter types are generated into `index.d.ts` by `napi build`
and shipped in the npm package; read them from your editor rather than from a
hand-maintained copy. The wire shapes match the MCP tools of the same name,
documented in the [MCP tool reference](../reference/MCP_TOOLS.md).

## Three contracts that apply to every method

**1. Everything is async and off-thread.** Each method schedules its work on
the libuv pool and resolves a `Promise`; the event loop is never blocked. The
one exception is the `MemoryService.open` factory, which is synchronous —
with a remote embedder (`"ollama"` or `"openai"`) it performs a single
blocking probe of the embedding endpoint at open time, to learn the vector
dimension; the default `"hash"` embedder does no I/O.

**2. Ids are decimal strings, never numbers.** A JS `number` is an f64 and
silently loses precision above 2^53, so every id (`remember`'s return,
`links[].target`, `relate`'s arguments, `fragment_id`, `content_hash`,
`memory_id`) crosses the boundary as a decimal string:

```js
const { id, similar } = await store.remember('a fact')  // id: '15545792975496669522' — a string
await store.relate(id, otherId, 'because')  // pass the string back verbatim
```

Passing a string that is not a decimal `u64` rejects with
`[INVALID_INPUT] invalid id '<value>' (expected a decimal u64 string)`.

**3. Errors are `Error`s prefixed with a stable code.** JavaScript has no
exception-class hierarchy to mirror the Python binding's typed errors, so the
category travels as a token at the front of the message:

| Code | Meaning | Typical trigger |
|---|---|---|
| `[INVALID_INPUT]` | Bad caller input | empty fact, oversized metadata, unknown filter operator, malformed id, a count (`k`, `ttlSeconds`, `maxHops`, `limit`, …) that is not a whole number in `0..=2^32-1` |
| `[NOT_FOUND]` | A referenced id or handle does not exist | `feedback` on an unknown id, `relate` to a missing endpoint, an expired `ctx://source/` handle |
| `[INTERNAL]` | Storage, embedding or extraction failure | store locked by another process, Ollama unreachable |

Branch on the prefix (`err.message.startsWith('[NOT_FOUND]')`); `err.code`
carries a coarser napi status for the same event.

## Choosing an embedder

```js
// Offline, deterministic, no network, no model. The default.
const offline = MemoryService.open('./agent_mem')

// Real semantic recall through a local Ollama instance.
const semantic = MemoryService.open(
  './agent_mem_ollama',
  'ollama',
  'http://localhost:11434', // default when omitted
  'all-minilm',             // default when omitted
)
```

`embedder="openai"` reaches any OpenAI-compatible server (oMLX, llama.cpp,
LM Studio, vLLM or a hosted provider); its URL, model and credential come from
the `VELESDB_MEMORY_EMBEDDER_*` environment only, never from an argument — see
[MCP_SERVER_SETUP.md](MCP_SERVER_SETUP.md#embedding-backend). Any other
embedder name rejects with `[INVALID_INPUT] unknown embedding backend '<name>'
(expected 'hash' for the offline deterministic embedder, 'ollama' for a local
model, or 'openai' for any OpenAI-compatible server — …)`.

A store is fixed to one embedder: the vector dimension is decided when the
store is created, so pointing an `ollama` service at a store first written
with `hash` (or the reverse) fails. Use a different directory instead.

## Auto-extraction (`rememberExtracted`)

Instead of calling `remember` once per fact, hand raw text to a local model
and let it split the text into atomic facts *and* wire the fact↔topic graph
that `why()` later walks:

```js
const outcome = await store.rememberExtracted(
  longText,
  'qwen3',                   // the Ollama model doing the extraction
  'http://localhost:11434',  // optional; this is the default
)
// outcome.ids — the stored fact ids; outcome.skippedOverCap — how many were dropped
```

The extractor is chosen per call with the `extractor` argument
(`'ollama'` default, or `VELESDB_MEMORY_EXTRACTOR` when set):

- `extractor: 'outline'` is deterministic and network-free — it reads the
  structure the passage states (`fact:` / `edge:` / `attr:` directives), so
  the whole contract is reachable with no model running.
- `extractor: 'openai'` speaks the OpenAI chat-completions protocol; URL and
  model come from the arguments or `VELESDB_MEMORY_EXTRACTOR_URL` /
  `VELESDB_MEMORY_EXTRACTOR_MODEL` (both required), and the credential from
  `VELESDB_MEMORY_EXTRACTOR_API_TOKEN` only — never from an argument.

Without a reachable backend it rejects with an `[INVALID_INPUT]` error rather
than crashing the process, and `skippedOverCap` tells you how many extracted
facts were dropped for exceeding the embeddable cap.

## File anchors (`remember`/`recall`)

With `VELESDB_MEMORY_INGEST_ROOTS` set at `open()` (a path-list of
allowlisted directories), a fact can be tied to the files it is about:

```js
const outcome = await store.remember(
  'the deploy broke after the cache change',
  [], null, null,
  ['notes/runbook.md'],   // anchors: repo-relative paths, at most 8
  checkoutRoot,           // anchorRoot: absolute, must sit inside an ingest root
)
const id = outcome.id
const hits = await store.recall('deploy broke', 5, null, checkoutRoot)
// each anchored hit carries freshness: { state: 'fresh' | 'stale' | 'unverified', changed: [...] }
```

The file digests are read under the allowlist at store time (a refused anchor
stores nothing); at recall time a changed or deleted file makes the fact
`stale` and ranks it lower — `score` stays the raw similarity. With no ingest
roots configured, anchors and `path` fragments are refused with
`[INVALID_INPUT]`, fail-closed.

## Context compilation (`compileContext`)

An agent burns most of its tokens re-reading redundant context.
`compileContext` compresses it **deterministically** — no LLM, no cloud: the
same request always compiles to the same bytes, duplicates drop, repeated log
lines collapse with counts, code / URLs / numbers / negative constraints
survive verbatim, and over-budget content becomes a recoverable
`ctx://source/` handle instead of a silent loss.

```js
const out = await store.compileContext({
  query: 'state of the canary deploy',
  token_budget: 4000,
  memory_scope: { k: 5 }, // optional: pull relevant stored memories in
  fragments: [
    { content: 'You are the deploy assistant.', metadata: { cache: true } },
    { content: ciLogs, kind: 'log' },
    { content: 'Never restart the primary during a rebalance.' },
  ],
})

out.content   // the compiled prompt context (fits the budget)
out.risk      // 'low' | 'medium' | 'high' — 'high' means critical content did not fit
out.decisions // one auditable decision per fragment (rule_id, reason, risk)
out.insights  // { tokens_in, tokens_out, tokens_saved, ... } — local estimates
```

`tokens_saved` is a local estimate, not billed tokens.

The request and result JSON match the MCP `compile_context` tool, with two
binding-wide differences:

- id fields (`fragment_id`, `content_hash`, `memory_id`, `fragment_ids`, and
  input `fragments[].id`) cross as decimal strings;
- the *top-level* result keys follow the binding's camelCase
  (`out.retrievalHandles`); nested trees keep the wire's snake_case.

Two read-only companions come with it. `explainCompilation(request,
fragmentId, fragmentIndex?)` re-compiles the request with event recording
forced off and returns the single decision for one fragment — it never counts
as a compilation. `contextSavings(project?)` aggregates the token and cost
estimates of past compilations, optionally narrowed to one project.

`fragmentIndex` (0-based) takes priority over `fragmentId` when both are
given: byte-identical fragments share a content-addressed id, so the index is
the only way to ask about the duplicate rather than the survivor.

The full rule set, budgets, `risk` semantics and preservation guarantees live
in the [context compiler guide](CONTEXT_COMPILER.md).

## Media fragments and `retrieveContextSource`

A fragment may carry an inline screenshot alongside its caption — set
`media: {mime, bytes_b64}` on it:

```js
const out = await store.compileContext({
  query: 'a screenshot of the failing build',
  token_budget: 4000,
  fragments: [
    { content: 'the failing build, before the fix',
      media: { mime: 'image/png', bytes_b64: pngB64 } },
  ],
})
```

The image packs atomically (never chunked, `rule_id: "media.atomic"`) and
costs tokens from its actual pixels, not from its base64 text — the full model
is in [Media fragments](CONTEXT_COMPILER.md#media-fragments).

`out.sources[i]` is a pointer only (`fragment_id` + `handle`). Fetch the
content itself back — inline or externalized by budget, it makes no
difference — with `retrieveContextSource`:

```js
const source = await store.retrieveContextSource(out.sources[0].handle)
source.content // the caption, byte for byte
source.media   // { mime, bytes_b64 } when the fragment carried one, else undefined
```

Same JSON shape as the MCP `retrieve_context_source` tool
(`{handle, content, media?}`). An unknown, expired or malformed handle
rejects with `[NOT_FOUND]` — never an internal error, never a crash.

## Transcripts and working contexts

`compileTranscript({query, transcript, token_budget, segmentation?})` is a
one-call shortcut over `compileContext` for a raw agent-session transcript. It
segments the text into turns (plain markers — `System:` / `User:` / `Human:` /
`Assistant:` / `AI:` / `Tool:` / `### User` / `### Assistant` — or JSONL, one
turn per line), then into code/log/body sub-segments, then compiles the
result. It resolves to `{context, segmentation}`, where `segmentation` audits
every cut (turn, role, kind, byte range, `fragment_id`) so you can see how the
transcript was split before trusting the output.

Like the MCP tool, the request takes `transcript` (inline) OR `path` (an
absolute file). `path` is resolved under the `VELESDB_MEMORY_INGEST_ROOTS`
allowlist read at `open()` — with no roots configured it is refused with
`[INVALID_INPUT]`, fail-closed. The same allowlist gates `path` fragments in
`compileContext`/`explainCompilation` and the file anchors of
`remember`/`recall` (see below).

Working contexts survive between sessions:

```js
await store.saveWorkingContext('veles', 'session-1', {
  goal: 'ship the Node surface',
  active_constraints: [{ text: 'never merge without green gates' }],
  pending_actions: ['load this back from a fresh MemoryService'],
})

// `{found, working, other_sessions}` — the same envelope the MCP tool serves.
const resumed = await store.loadWorkingContext('veles', 'session-1')
const { sessions } = await store.listWorkingContexts('veles')        // [] when the project never saved
```

**Breaking in 0.12.0**: `loadWorkingContext` used to resolve the bare working
context, or `null`. Read `resumed.working` for that value. The bare form
collapsed two different answers into one — a project that never saved
anything, and a typo in `session` that missed a session which *does* exist.
`resumed.other_sessions` is what tells them apart, and it is filled in on a
hit too: a typo landing on another real session returns `found: true`, the
case a caller can least detect on its own.

Saving again under the same `project` + `session` replaces the previous state
(idempotent upsert). Id fields nested inside the working context follow the
same decimal-string contract in both directions — including the ones under
`resumed.working`, one level deeper than before.

## Bundled agent skills

The npm package ships four skills that teach an agent *when* to call these
methods — wiring the API alone gives it the verbs, not the loop:

| Skill | What it teaches |
|---|---|
| `skills/velesdb-memory` | recall before acting → remember decisions with metadata **and** links → `relate` as relationships appear → `why` to explain → `feedback` to reinforce |
| `skills/velesdb-context-optimizer` | the full compression workflow, including when *not* to compress |
| `skills/velesdb-extract-outline` | the `outline` directive grammar (`fact:`/`edge:`/`attr:`) — agent-side extraction without a generative model, for no-LLM and small-model setups the user opts into |
| `skills/velesdb-learning-loop` | the discipline over both: recall before designing, check recurrence before storing a fix, correct a wrong memory instead of adding beside it, and never write by reflex |

```bash
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-memory ~/.claude/skills/
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-context-optimizer ~/.claude/skills/
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-extract-outline ~/.claude/skills/
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-learning-loop ~/.claude/skills/
```

**Keep them fresh.** That `cp` is a snapshot, not a live link — re-run it
after every `npm update` so the installed skill picks up the doc and behaviour
changes of the new package version. Repeating it is safe: it overwrites the
local copy.

## Need the full engine?

This addon is the **memory wedge** by design: memory semantics only. It does
not expose raw VelesQL, deep graph `MATCH`, collection administration, or any
other database-shaped capability — that would cross the
[VelesDB Core License 1.0](https://github.com/cyberlife-coder/VelesDB/blob/develop/LICENSE)
§1 "Substantial Set" line.

For the full engine from Node or TypeScript, run the REST server and talk to
it with [`@wiscale/velesdb-sdk`](https://www.npmjs.com/package/@wiscale/velesdb-sdk):

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

The full REST API (VelesQL, graph `MATCH`, auth, TLS) is in the
[server README](../../crates/velesdb-server/README.md); the REST-backend
client surface is in the
[TypeScript SDK README](../../sdks/typescript/README.md).

## Resource caps

These caps are enforced in `velesdb-memory`'s shared `limits` module, so the
addon, the MCP server and the Python binding all apply the same numbers.
Exceeding a hard cap rejects with `[INVALID_INPUT]`; a clamped value is
silently reduced instead.

| Cap | Value | Behaviour |
|---|---|---|
| Fact size (`remember`, `rememberExtracted`) | 1 MiB | rejects |
| Metadata size, serialized as JSON | 64 KiB | rejects |
| `recall` / `recallWhere` / `recallFused` limit `k` | 1000 | clamped |
| `why` hops | 10 (default 2) | clamped |
| Fragments per compile request | 1024 | rejects |
| Fragment `content` size | 1 MiB | rejects |

### JSON arguments: metadata, filters, requests, working contexts

These arguments must be JSON data, what `JSON.parse` can produce: `null`,
booleans, finite numbers, strings, arrays and plain objects (an object
literal, or `Object.create(null)`). Anything else rejects with
`INVALID_INPUT`, naming where it sits (`metadata.when is a Date, not a plain
object`): a `Date`, a `Map`, a `Set`, a class instance, a typed array, a
function, a symbol, a `bigint`, `NaN` or an infinity. They used to be stored
as `{}` (or `null`) without a word; convert them first (`date.toISOString()`,
`Object.fromEntries(map)`, a decimal string for a `bigint`). As with
`JSON.stringify`, a property set to `undefined` is left out.

### Numbers in metadata and filters

A JS `number` that holds a whole value — up to `Number.MAX_SAFE_INTEGER`
(2^53 − 1) — is stored as an integer, whatever its size: `{ ts: 5000000000 }`
reads back as the integer `5000000000` from Python or Rust, and an exact
metadata filter on it matches what the other bindings stored. Fractions, and
whole numbers past 2^53 − 1 (which JS cannot hold exactly), stay floats.

### Argument errors

A value the binding refuses — a JSON argument that is not JSON data, a count
out of range, a malformed id — rejects the returned `Promise` with an
`[INVALID_INPUT]` error. An argument of the wrong JS type for a typed
parameter (a number where a string is expected) is refused by napi before the
method runs: that call throws synchronously, with napi's own `code`
(`StringExpected`, `NumberExpected`, …) and no `[CODE]` prefix.
The same holds for a JSON argument whose getter or `Proxy` trap throws while
the binding reads it: napi offers no safe way to take that pending JS
exception back, so it surfaces synchronously, as the getter's own error.

---

Last updated: 2026-08-09 · Applies to: velesdb-core 8.2.0
