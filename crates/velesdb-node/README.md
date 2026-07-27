# velesdb-node — `@wiscale/velesdb-memory-node`

> Local-first agent memory for Node.js: remember, recall, and explain *why* — in-process, no server.

[![npm](https://img.shields.io/npm/v/%40wiscale%2Fvelesdb-memory-node?logo=npm&label=npm)](https://www.npmjs.com/package/@wiscale/velesdb-memory-node)
[![Node](https://img.shields.io/node/v/%40wiscale%2Fvelesdb-memory-node?logo=nodedotjs&label=node)](https://www.npmjs.com/package/@wiscale/velesdb-memory-node)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0_(source--available)-e8702a)](./LICENSE)

> **Portability**: ✅ a plain npm dependency — no MCP client, no server, no
> daemon, no API key · ✅ prebuilt binaries for macOS, Linux (glibc) and
> Windows · ⚠️ no musl prebuild, so Alpine-based images need a
> [local build](../../docs/guides/NODE_ADDON_BUILD.md).

This crate is never published to crates.io. It compiles to a napi-rs `cdylib`
that ships to npm as `@wiscale/velesdb-memory-node`, wrapping the exact same
hardened Rust as the [velesdb-memory](../velesdb-memory/README.md) MCP server
and the Python binding — no logic is reimplemented here.

## Objective

An agent that runs in Node forgets everything between processes, and a vector
store only hands back text that *looks like* the question. Neither can answer
"why is this value 7?", because the answer is usually a fact that shares no
words with the question — the customer constraint behind the constant, the
incident behind the config.

This addon gives a Node agent durable memory that never leaves the machine.
It remembers facts, recalls them semantically, **connects** them with typed
links, and walks those links to return the evidence trail behind an answer.
It also carries the deterministic context compiler, which shrinks a prompt
under a hard token budget with no model call at all.

![recall() finds the booking but misses the reason; why() reaches it through typed links, across a session restart](https://raw.githubusercontent.com/cyberlife-coder/VelesDB/develop/examples/agent_memory/why_across_sessions.gif)

> The store is on disk, so memory survives process restarts: a new session
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

## Use cases

- A Node or TypeScript coding agent that must still know, three weeks and
  several processes later, *why* a timeout is 7 seconds — and can show the
  constraint it came from.
- An Electron or CLI tool that needs memory without asking the user to
  install, run and secure a database service.
- Regulated or air-gapped work where context cannot transit a third-party LLM
  API, and "show why it recalled that" has to be answerable.
- A long agent session about to blow its context window: compile the prompt
  under a budget instead of summarizing and restarting.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Node.js | 18.17 | The package `engines.node` floor. CI builds and tests on Node 20. |
| A supported platform | — | macOS (arm64/x64), Linux glibc (x64/arm64), Windows x64 — see [Compatibility](#compatibility). |
| Rust | 1.90 | **Not needed to install.** Only to build the addon yourself: [building from source](../../docs/guides/NODE_ADDON_BUILD.md). |
| Ollama | any | **Optional.** Only for `embedder: "ollama"` and `rememberExtracted`. The default embedder is offline and dependency-free. |

## Installation

```bash
npm install @wiscale/velesdb-memory-node
```

That downloads a prebuilt binary; nothing is compiled on your machine, and no
Rust toolchain is involved. Unsupported platform, or working on the binding
itself? See [building the Node addon from
source](../../docs/guides/NODE_ADDON_BUILD.md).

## First success in 60 seconds

Save this as `first.mjs` in a project with `"type": "module"`, then run
`node first.mjs`:

```js
import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { MemoryService } from '@wiscale/velesdb-memory-node'

// Offline "hash" embedder by default: no model, no network, no API key.
const store = MemoryService.open(mkdtempSync(join(tmpdir(), 'velesdb-')))

// An earlier session recorded a decision and the human reason behind it.
const reason = await store.remember(
  'field crews work from remote mining sites over satellite links',
)
await store.remember('the default HTTP request timeout is 7 seconds', [
  { target: reason, relation: 'because' },
])
await store.remember('the vector index uses HNSW with M=16')

const question = 'why is the request timeout 7 seconds?'

console.log('recall — vector similarity only:')
for (const hit of await store.recall(question, 2)) {
  console.log(`  ${hit.content}`)
}

console.log('why  — vector seed + graph of typed links:')
const { nodes, edges } = await store.why(question)
for (const node of nodes) console.log(`  hop ${node.hop}  ${node.content}`)
console.log(`  ${edges.length} typed edge(s) walked`)
```

Expected output, exactly:

```text
recall — vector similarity only:
  the default HTTP request timeout is 7 seconds
  the vector index uses HNSW with M=16
why  — vector seed + graph of typed links:
  hop 0  the default HTTP request timeout is 7 seconds
  hop 1  field crews work from remote mining sites over satellite links
  1 typed edge(s) walked
```

That is the whole product in eight lines of output. **`recall` never surfaces
the satellite-link constraint** — it shares no words with the question, so
vector similarity ranks an unrelated HNSW note above it. `why()` follows the
`because` edge and reaches it at hop 1, so your agent warns you *before* you
"round 7 down" and cut off the customer.

Failure looks unmistakable: `Failed to load native binding` means no prebuilt
binary matched your platform ([Troubleshooting](#troubleshooting)), and a
`why` section with only `hop 0` means the typed link was not stored — check
that `reason` was passed as `{ target, relation }`.

## Configuration

There are no environment variables. Everything is an argument to the factory:

| Argument | Default | Effect |
|---|---|---|
| `path` | — | Directory of the on-disk store. Created if missing; reopened on the next run. |
| `embedder` | `"hash"` | `"hash"` is offline and deterministic; `"ollama"` gives real semantic recall. |
| `ollamaUrl` | `"http://localhost:11434"` | Only with `embedder: "ollama"`. |
| `ollamaModel` | `"all-minilm"` | Only with `embedder: "ollama"`. |

```js
const store = MemoryService.open('./agent_mem', 'ollama')
```

A store is fixed to one embedder — the vector dimension is decided when the
store is created — so use a separate directory when you switch.

## Examples

[`examples/why_magic_constant.mjs`](examples/why_magic_constant.mjs) is the
runnable version of the demo above, scaled to 14 memories so the blindness of
plain recall is unmistakable. From this directory, after a build:

```bash
node examples/why_magic_constant.mjs
```

The same wedge in the other bindings is listed in the
[velesdb-memory README](../velesdb-memory/README.md#see-the-wedge-offline-one-command).

## API

18 methods on one class, in three families:

| Family | Methods |
|---|---|
| Durable memory | `remember`, `recall`, `recallWhere`, `recallFused`, `recallFusedDated`, `relate`, `forget`, `why`, `feedback`, `rememberExtracted` |
| Context compiler | `compileContext`, `compileTranscript`, `explainCompilation`, `contextSavings`, `retrieveContextSource` |
| Session resumption | `saveWorkingContext`, `loadWorkingContext`, `listWorkingContexts` |

Three contracts hold across all of them: every method returns a `Promise` and
runs off the event-loop thread; every id crosses as a **decimal string** (a JS
`number` loses precision above 2^53); every rejection is an `Error` whose
message starts with `[INVALID_INPUT]`, `[NOT_FOUND]` or `[INTERNAL]`.

Parameter types are generated into `index.d.ts` and shipped with the package —
read them from your editor. Everything else (per-method semantics, the
compiler surface, media fragments, working contexts, resource caps) is in the
**[Node addon guide](../../docs/guides/NODE_ADDON.md)**.

## Bundled agent skills

Wiring the API gives your agent the *methods*; it does not tell it *when* to
use them. Two skills ship inside the package for that — `velesdb-memory` (the
recall → remember → relate → why → feedback loop) and
`velesdb-context-optimizer` (the compression workflow, including when *not* to
compress):

```bash
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-memory ~/.claude/skills/
cp -r node_modules/@wiscale/velesdb-memory-node/skills/velesdb-context-optimizer ~/.claude/skills/
```

That `cp` is a snapshot, not a live link: re-run it after every `npm update`.
Details in the [Node addon guide](../../docs/guides/NODE_ADDON.md#bundled-agent-skills).

## Need the full engine?

This addon is the **memory wedge**, by design and by license: memory
semantics only. It exposes no raw VelesQL, no deep graph `MATCH`, no
collection administration — a test pins the prototype allowlist and asserts
`query`, `upsert`, `createCollection` and `traverse` are absent.

For the full engine from Node, run the REST server and talk to it with
[`@wiscale/velesdb-sdk`](https://www.npmjs.com/package/@wiscale/velesdb-sdk);
the runnable two-step recipe is in the
[Node addon guide](../../docs/guides/NODE_ADDON.md#need-the-full-engine).

## Known limits

- **Memory semantics only.** No database-shaped API, ever — see above.
- **One process per store.** The store takes a single-writer lock, so a second
  `MemoryService.open` on the same directory fails while the first is alive.
- **A store is fixed to one embedder.** The dimension is set at creation.
- **No `path` fragment ingestion.** The MCP server can read a file by
  reference under an allowlist; this binding has no such configuration
  surface. Read the file yourself and pass its content.
- **Bring-your-own-links by default.** The graph comes from `relate` and
  `links`; automatic extraction needs `rememberExtracted` and a local model.
- **No musl prebuild**, so Alpine images must build the addon themselves.
- **Not on crates.io.** `publish = false`: the artifact is the npm package.

## Compatibility

Prebuilt binaries, one per target declared in `package.json` `napi.targets`:

| Platform | Target triple | Status |
|---|---|---|
| macOS, Apple silicon | `aarch64-apple-darwin` | Prebuilt, load-smoke-tested in CI |
| macOS, Intel | `x86_64-apple-darwin` | Prebuilt, cross-built (no native runner to smoke-test on) |
| Linux x64, glibc | `x86_64-unknown-linux-gnu` | Prebuilt, load-smoke-tested in CI |
| Linux arm64, glibc | `aarch64-unknown-linux-gnu` | Prebuilt, cross-built |
| Windows x64 | `x86_64-pc-windows-msvc` | Prebuilt, load-smoke-tested in CI |
| Linux musl (Alpine) | `*-unknown-linux-musl` | **Not shipped** — [build from source](../../docs/guides/NODE_ADDON_BUILD.md) |

| Runtime | Status |
|---|---|
| Node.js 18.17+ | Supported (`engines.node`) |
| Node.js 20 | The version CI builds and tests on |
| Bun / Deno | Untested — Node-API support exists in both, but nothing here verifies it |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Failed to load native binding` | No prebuilt binary matches this platform — most often Alpine/musl. There is no source fallback. | [Build the addon from source](../../docs/guides/NODE_ADDON_BUILD.md), or use a glibc base image. |
| `[INTERNAL] storage error: [VELES-031] Database is already opened by another process: <path>` | The single-writer lock is held — a second `MemoryService` on the same directory. | Keep one instance per store, or give the second one its own path. |
| `[NOT_FOUND] memory 999999999 does not exist` on `relate` / `feedback` | The id was rounded by JS number arithmetic, or came from a different store. | Never convert an id with `Number()`; pass the decimal string through verbatim. |
| `[INVALID_INPUT] invalid id 'not-an-id' (expected a decimal u64 string)` | A non-numeric value reached an id argument (often an object or `undefined`). | Pass the exact string `remember` resolved to. |
| `[INTERNAL] extraction error: ... ollama request failed: ... Connection refused` | `rememberExtracted` needs a running Ollama, whatever embedder the store uses. | Start Ollama and pull the model, or use `remember` with explicit `links`. |
| `EPERM` / `EBUSY` deleting a store directory on Windows | The `velesdb.lock` file is still held; the release finalizer is not deterministic. | Retry the delete, or drop the directory on the next run. |

## License

VelesDB Core License 1.0 (source-available, based on ELv2). See
[LICENSE](./LICENSE) — a local copy, so npm bundles it into the published
package and each per-platform sub-package.

Running this addon inside your own application, where your users only ever
receive results, is the license's expressly-permitted **embedded, local-first
use**. What it forbids is re-hosting VelesDB as a multi-tenant service where
third parties drive the database — which this package makes impossible by
construction: memory semantics only, and it is a library, not a service.
Questions: contact@wiscale.fr.

---

`velesdb-node v0.11.2` (npm `@wiscale/velesdb-memory-node@0.11.1`) · Last updated: 2026-07-25 · Applies to: velesdb-core 4.1.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
