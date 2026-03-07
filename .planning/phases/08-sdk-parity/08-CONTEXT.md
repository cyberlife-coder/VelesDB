# Phase 8: SDK Parity - Context

**Gathered:** 2026-03-07
**Status:** Ready for planning

<domain>
## Phase Boundary

All SDK and integration surfaces expose the full v1.5 API — Python, TypeScript, WASM, Mobile, LangChain, LlamaIndex, and the Tauri plugin all support sparse vectors, PQ config, and streaming inserts (where applicable). No new core engine work — this phase wires existing v1.5 features into SDK bindings.

Requirements: SDK-01, SDK-02, SDK-03, SDK-04, SDK-05, SDK-06, SDK-07

</domain>

<decisions>
## Implementation Decisions

### Python SDK — sparse vectors (SDK-01)
- **Dual input format** for sparse vectors: accept both native Python `dict[int, float]` AND `scipy.sparse` matrices (scipy as optional dependency)
- **Unified search()** method: `collection.search(vector=v, sparse_vector=sv, top_k=10)` — hybrid dense+sparse is implicit when both are provided; sparse-only when only `sparse_vector` is passed
- **Sparse in upsert**: `sparse_vector` field in the point dict — `collection.upsert([{'id': 1, 'vector': [...], 'sparse_vector': {42: 0.8}, 'payload': {...}}])`
- No separate `sparse_search()` or `upsert_sparse()` methods

### Python SDK — PQ & streaming (SDK-01)
- **Methods on Collection**: `collection.train_pq(m=8, k=256)` and `collection.stream_insert(points)`
- Not exposed via VelesQL-only — native Python methods are the primary interface
- `stream_insert()` maps to `POST /collections/{name}/stream/insert` under the hood

### TypeScript SDK (SDK-02)
- **Same pattern as Python**: `sparseVector` optional field in `VectorDocument`, `search()` accepts optional `sparseVector`, `trainPq()` and `streamInsert()` on the client
- **REST backend**: wires to existing REST endpoints (sparse upsert/search, stream insert, PQ train)
- **WASM backend**: supports sparse search (module `sparse.rs` already exists) — no streaming (no tokio), no PQ training
- **streamInsert()**: simple async method, POST to stream endpoint, returns accepted or throws on 429 backpressure — no WebSocket, no callback

### WASM module (SDK-03)
- Sparse search supported without `persistence` feature — `sparse.rs` already in velesdb-wasm
- Plan cache active (already gated correctly in Phase 7)
- No streaming insert (requires tokio), no PQ training (requires persistence for disk I/O)
- Build verification: `cargo build -p velesdb-wasm --no-default-features --target wasm32-unknown-unknown`

### Mobile iOS/Android (SDK-04)
- **Sparse + PQ only** — no streaming insert (streaming is a server use case, not mobile)
- UniFFI bindings updated for: sparse upsert, sparse search, PQ train, PQ config
- **Sparse vector format**: `HashMap<u32, f32>` — UniFFI maps to Swift `Dictionary<UInt32, Float>` and Kotlin `Map<UInt, Float>`

### LangChain VectorStore (SDK-05)
- **Examples in `examples/langchain/`** — not a separate published package
- VelesDBVectorStore wraps the Python SDK (`import velesdb`)
- **Hybrid dense+sparse native**: `add_texts()` accepts sparse vectors, `similarity_search()` does hybrid when sparse embeddings available
- Demonstrates the value proposition of VelesDB (single-engine hybrid search)

### LlamaIndex integration (SDK-06)
- **Examples in `examples/llamaindex/`** — not a separate published package
- Same pattern as LangChain: wraps Python SDK, hybrid dense+sparse supported
- Shows PQ config via the VelesDB VectorStore

### Tauri plugin (SDK-07)
- **Full parity**: sparse upsert/search, PQ train/config, AND streaming insert
- Tauri runs on desktop with tokio available — streaming is viable
- Sparse vector format: serde JSON `{"42": 0.8}` (string keys in JSON, parsed to u32)

### Cross-SDK consistency
- **HashMap<u32, f32>** as the canonical sparse vector format across all surfaces (dict in Python, object in JS/TS, HashMap in Rust/UniFFI, JSON object in REST/Tauri)
- Python additionally accepts scipy.sparse as convenience input (converted internally)
- Method naming follows each language's conventions: `train_pq` (Python), `trainPq` (TypeScript), `train_pq` (Rust/Tauri command)

### Claude's Discretion
- Internal conversion logic for scipy.sparse to dict in Python
- TypeScript type definitions for sparse vector (Record<number, number> vs custom type)
- WASM sparse search API surface (method names, return types)
- UniFFI sparse vector ergonomics (whether to add convenience methods)
- LangChain/LlamaIndex example structure and completeness level
- Error handling patterns per SDK (how 429 backpressure is surfaced)

</decisions>

<specifics>
## Specific Ideas

- Python search() unification: one entry point for dense, sparse, and hybrid — the presence of vector/sparse_vector determines the mode automatically
- LangChain example should demonstrate the "single engine, no glue code" value prop — one VelesDB collection doing both dense and sparse without separate indices
- WASM module already has sparse.rs and quantization.rs — leverage existing code, don't rewrite
- Tauri streaming maps to the same REST endpoint pattern as the server

</specifics>

<code_context>
## Existing Code Insights

### Reusable Assets
- `crates/velesdb-python/src/collection.rs`: Collection wrapper with upsert/search — extend with sparse_vector parameter
- `crates/velesdb-python/src/collection_helpers.rs`: Point parsing, search result conversion — add sparse vector parsing
- `crates/velesdb-wasm/src/sparse.rs`: Sparse index already implemented for WASM — wire into VectorStore API
- `crates/velesdb-wasm/src/quantization.rs`: Quantization support in WASM — already available
- `sdks/typescript/src/types.ts`: TypeScript type definitions — add SparseVector, PQ config types
- `sdks/typescript/src/backends/rest.ts`: REST backend — add sparse/streaming/PQ endpoints
- `sdks/typescript/src/backends/wasm.ts`: WASM backend — wire sparse search
- `crates/velesdb-mobile/src/types.rs`: UniFFI types — add sparse vector and PQ config types
- `crates/tauri-plugin-velesdb/src/commands.rs`: Tauri commands — add sparse/PQ/streaming commands

### Established Patterns
- PyO3 `#[pymethods]` on Collection for all operations (search, upsert, etc.)
- UniFFI `#[uniffi::export]` for mobile bindings
- Tauri `#[tauri::command]` for plugin commands
- TypeScript SDK: `IVelesDBBackend` interface with REST and WASM implementations
- WASM: `#[wasm_bindgen]` exports with serde for complex types

### Integration Points
- `velesdb-core` Collection API: sparse_search(), train_pq(), stream_insert() — already implemented in phases 3-7
- REST server endpoints: sparse upsert/search (Phase 5), stream insert (Phase 7), PQ train (Phase 3)
- `Point` struct: `sparse_vector: Option<BTreeMap<String, SparseVector>>` — already has sparse field

</code_context>

<deferred>
## Deferred Ideas

None — discussion stayed within phase scope

</deferred>

---

*Phase: 08-sdk-parity*
*Context gathered: 2026-03-07*
