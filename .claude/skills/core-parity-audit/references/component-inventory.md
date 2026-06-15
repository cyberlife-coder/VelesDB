# Component inspection hints

How each child exposes (or should expose) core's surface, and what counts as `full | partial | absent | na`. Re-verify paths each run — modules move.

## Rust crates (delegate to `velesdb_core::` directly)

- **server** (`crates/velesdb-server`): HTTP adapters in `src/routes.rs`, `src/handlers/`, `src/types.rs`. `full` = a REST endpoint exposes it. `na` = pure Rust accessor with no REST meaning. Enrichment seen: `/health`, `/ready`, `/metrics` (Prometheus), `/vacuum`, `/compact`, SSE traversal stream, bearer-auth/TLS/rate-limit.
- **cli** (`crates/velesdb-cli`): `src/repl_*_cmds.rs`, `src/commands.rs`, `src/handlers/`, `src/import.rs`, `src/repl_execute.rs`. Embedded core → VelesQL paths broadly reachable. **Agent memory has no CLI surface by design → `na`, not `absent`** (don't let the mapper over-report).
- **python** (`crates/velesdb-python`): `#[pymethods]`/`#[pyfunction]` in `src/*.rs` **and** `python/velesdb/__init__.pyi`. `full` only if in **both** binding and stub. Enrichment: NumPy/DataFrame fast paths, `train_pq`, embedders.
- **wasm** (`crates/velesdb-wasm`): `#[wasm_bindgen]` exports. Built **without `persistence`** → persistence/WAL/HNSW/streaming are `na` by design; it reimplements an in-memory store + VelesQL executor (`velesql_exec*.rs`) + brute-force search. Enrichment: IndexedDB persistence, Web-Worker traversal offload.
- **mobile** (`crates/velesdb-mobile`): `#[uniffi::export]`. `full` = exported to Swift/Kotlin. Semantic-only agent memory by design. Enrichment: `MobileGraphStore`, `compact_storage`, `train_pq`.
- **migrate** (`crates/velesdb-migrate`): ETL tool — most data/query rows are legitimately `na`. Focus on whether it writes through core APIs (`upsert_bulk`) and provisions indexes/analyze post-load. Enrichment: 10 source connectors, resumable checkpoints, retry/backoff.
- **tauri** (`crates/tauri-plugin-velesdb`): `#[tauri::command]` handlers. Enrichment: event stream, `get_app_data_dir`, persisted memory snapshots.

## Clients / integrations

- **ts-sdk** (`sdks/typescript`): `src/client.ts`, `src/client/`, `src/backends/`, `src/agent-memory.ts`, `src/query-builder.ts`, `src/types.ts`, `src/capabilities.ts`. **Pure client** — `full` = it builds the right wire payload / parses the response. The query-builder/filter DSL compiling to wire is clean. `concern` if it computes *results* (ranking/fusion/distance) itself, or hardcodes enum/`VELES-###` contracts that can drift.
- **langchain** (`integrations/langchain`): `src/langchain_velesdb/*.py`. **MIT** — must reuse `integrations/common`, call the `velesdb` binding, never embed protected logic. Enrichment: MMR, graph-toolkit (LLM extraction), retrievers.
- **llamaindex** (`integrations/llamaindex`): `src/llamaindex_velesdb/*.py`. Same MIT rule. Enrichment: score-threshold query, graph retrievers.
- **haystack** (`integrations/haystack`): `src/haystack_velesdb/document_store.py`. Bounded by Haystack 2.x `DocumentStore` protocol (`write_documents`/`filter_documents`/`embedding_retrieval`/`count`/`delete`). Graph/agent-memory/sparse-named are `na` by protocol. **Watch:** its own `_str_id_to_int` may diverge from `velesdb_common.ids.stable_hash_id`.
- **common** (`integrations/common`): `src/velesdb_common/*.py` — the shared MIT base (`ids`, `fusion`, `graph`, `security`, `collection_admin`). Verify the 3 integrations actually reuse it instead of re-deriving id-hashing/fusion/security per-package.

## Docs

`docs/reference/api-reference.md`, `VELESQL_CONTRACT.md`, `VELESQL_CHEATSHEET.md`, `ECOSYSTEM_PARITY.md`, `docs/guides/`, `README.md`. `full` = capability documented with usage; `partial` = mentioned but thin; `absent` = undocumented public capability (a doc gap); `na` = purely internal symbol. Reconcile findings against the `ECOSYSTEM_PARITY.md` "Remaining Gaps" list.
