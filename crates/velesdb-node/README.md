# @wiscale/velesdb-memory-node

Local-first **agent memory** for Node.js — the VelesDB memory wedge as an
in-process native addon (napi-rs). Same hardened Rust as the MCP server and the
Python binding; no network service.

`remember` / `recall` / `recallWhere` / `relate` / `forget` / `why` /
`rememberExtracted`. The differentiator is **`why()`**: it answers a question with
the best-matching memory *plus its connected subgraph* — related facts a plain
vector recall is blind to.

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

## License

VelesDB Core License 1.0 (based on ELv2). See [LICENSE](./LICENSE). This addon
exposes memory semantics only; it is not a hosted or managed service.
