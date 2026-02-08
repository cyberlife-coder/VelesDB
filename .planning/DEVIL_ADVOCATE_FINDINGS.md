# 🔥 Devil's Advocate Code Review — VelesDB-Core

> **Reviewer stance:** "I'm a senior Rust expert. I ignore ALL comments and docs. I only read code."
>
> **Date:** 2025-01-XX
> **Branch:** develop (pre-merge to main)

---

## Severity Legend

| Level | Meaning |
|-------|---------|
| 🚨 **CRITICAL** | Code lies about what it does, produces wrong results |
| 🐛 **BUG** | Incorrect logic that will cause wrong behavior |
| ⚠️ **DESIGN** | Structural problems that hurt perf/maintainability |
| 📝 **MINOR** | Code smell, dead code, naming issues |

---

## 🚨 CRITICAL — Code That Lies

### C-01: GPU batch_euclidean_distance / batch_dot_product are CPU loops

**File:** `src/gpu/gpu_backend.rs` (lines ~362-420)

`GpuAccelerator::batch_euclidean_distance()` and `batch_dot_product()` iterate vectors on CPU calling `simd_native`. Only `batch_cosine_similarity()` has a real WGSL compute shader. **2/3 GPU methods are fake.**

**Impact:** Users selecting GPU acceleration for Euclidean/DotProduct get CPU performance. Benchmarks comparing GPU vs CPU for these metrics are meaningless.

### C-02: GpuTrigramAccelerator = zero GPU code

**File:** `src/index/trigram/gpu.rs` (lines ~68-80)

`batch_search()` and `batch_extract_trigrams()` both call CPU functions via `.iter().map()`. The struct name `GpuTrigramAccelerator` is a lie — no GPU kernels exist for trigram operations.

**Impact:** False advertising. Users enabling the `gpu` feature for trigram search get zero benefit.

### C-03: GPU brute force search hardcodes cosine metric

**File:** `src/index/hnsw/index/search.rs` (line ~261)

`search_brute_force_gpu()` always calls `batch_cosine_similarity()` regardless of the index's configured distance metric (Euclidean, DotProduct, Hamming, Jaccard).

**Impact:** **Wrong results** for any non-cosine index using GPU brute force. Rankings will be incorrect.

### C-04: RRF (Reciprocal Rank Fusion) formula is mathematically wrong

**File:** `src/collection/search/query/score_fusion/mod.rs` (line ~209)

Real RRF: `Σ 1/(k + rank_i)` where `rank` is positional rank in a sorted list.
Implementation: `Σ 1/(k + (1.0 - score) * 100.0)` — uses score as a proxy for rank.

This is a completely different formula. A cosine of 0.95 gives pseudo-rank=5, cosine of 0.50 gives pseudo-rank=50. The theoretical rank-fusion properties of RRF (robustness to outlier scores, normalization across heterogeneous score distributions) are absent.

**Impact:** Users choosing RRF fusion strategy get a proprietary formula, not the standard RRF from the literature.

---

## 🐛 BUG — Incorrect Logic

### B-01: VelesQL vector parameters allow NaN/Infinity

**File:** `src/collection/search/query/extraction.rs` (lines ~116-122)

The `else` branch casts NaN/Infinity f64 to f32 without rejection. Vectors containing NaN or Infinity components pass through to HNSW search, producing indeterminate distance calculations.

**Impact:** Corrupted search results for queries with non-finite vector components. Silent failure.

### B-02: ORDER BY property paths is a silent no-op

**File:** `src/collection/search/query/match_exec/similarity.rs` (lines ~204-208)

`order_match_results()` catch-all `_` branch for property paths (e.g., `ORDER BY n.name DESC`) does nothing. No error, no warning.

**Impact:** Users believe results are ordered by a property but get non-deterministic ordering.

### B-03: Weighted fusion = Average (identical implementations)

**File:** `src/collection/search/query/score_fusion/mod.rs` (lines ~215-221)

`Weighted` strategy uses `1.0 / scores.len()` as weight for all components — exactly identical to `Average`. No actual weight configuration mechanism exists.

**Impact:** Users selecting `Weighted` over `Average` get the same results. The API promises configurable weights but delivers none.

### B-04: DualPrecisionHnsw::search() doesn't use quantized distances

**File:** `src/index/hnsw/native/dual_precision.rs` (lines ~209-243)

`search_dual_precision()` calls `self.inner.search()` (standard float32 HNSW), then re-ranks with the same float32 vectors. The quantized store is never used. Only `search_with_config()` with explicit `DualPrecisionConfig` triggers real int8 traversal.

**Impact:** Default dual-precision search is just "overfetch + rerank with same precision" — no actual dual-precision benefit. The 4x bandwidth reduction is only available through the non-default API.

### B-05: BfsIterator visited_overflow clears entire visited set

**File:** `src/collection/graph/streaming.rs` (lines ~207-210)

When `visited.len() >= max_visited_size`, the code calls `self.visited.clear()` and sets `visited_overflow = true`. This means ALL previously tracked nodes become unvisited again.

**Impact:** In cyclic graphs, previously visited nodes can be re-traversed, causing duplicate results and exponential blowup (bounded only by `max_depth`). Fix: stop inserting but do NOT clear.

### B-06: cosine_similarity_quantized full dequantization for norm

**File:** `src/quantization/scalar.rs` (lines ~197-199)

`cosine_similarity_quantized()` calls `quantized.to_f32()` to reconstruct the entire f32 vector, just to compute its norm. This allocates `dimension * 4` bytes per call, negating quantization's memory benefit.

**Impact:** Performance regression. Each cosine similarity call on SQ8 vectors allocates and computes on full f32 size.

---

## ⚠️ DESIGN — Structural Problems

### D-01: ColumnStore dual deletion tracking (FxHashSet + RoaringBitmap)

**File:** `src/column_store/mod.rs` (lines ~64-67)

`deleted_rows` (FxHashSet<usize>) and `deletion_bitmap` (RoaringBitmap) track the same information. ALL filter operations use only `deleted_rows`. The bitmap is dead weight that must be kept in sync.

**Impact:** Wasted memory, synchronization risk, code complexity.

### D-02: HNSW search layer lock contention

**File:** `src/index/hnsw/native/graph/search.rs` (lines ~99, ~159)

`self.layers.read()` is acquired and dropped on every iteration of `search_layer_single` and `search_layer`. During concurrent inserts, this creates RwLock ping-pong.

**Impact:** Latency spikes under concurrent read/write workloads. The vectors lock is held for the entire search, but layers lock is re-acquired hundreds of times.

### D-03: CART Node4 dead code + leaf splitting absent

**File:** `src/collection/graph/cart/node.rs`

- `Node4` variant is defined but never constructed (`#[allow(dead_code)]`)
- Leaf node `insert()` has TODO for leaf splitting — leaves grow unbounded

**Impact:** Unbounded leaf growth degrades to linear scan for high-cardinality prefixes.

### D-04: Over-fetch factor hardcoded at 10x

**File:** `src/collection/search/query/mod.rs` (line ~188)

`overfetch_factor = 10 * similarity_conditions.len().max(1)` — arbitrary constant. No adaptive mechanism.

**Impact:** 10x insufficient for high-selectivity filters (missed results), wasteful for low-selectivity.

### D-05: WAL has no per-entry CRC

**File:** `src/storage/log_payload.rs`

Snapshots have CRC32 validation, but individual WAL entries do not. Bit-flips in WAL entries are only detected at read time (JSON parse failure), not during recovery replay.

**Impact:** Structurally valid but corrupted payloads can be indexed and served. Silent data corruption.

### D-06: LogPayloadStorage::store flushes after every write

**File:** `src/storage/log_payload.rs` (line ~400)

`wal.flush()` called after every single `store()` operation. Good for durability, terrible for throughput.

**Impact:** Write throughput bottleneck. Batch inserts pay per-entry flush overhead.

### D-07: LogPayloadStorage::should_create_snapshot takes write lock

**File:** `src/storage/log_payload.rs` (line ~367)

`self.wal.write().get_ref().metadata()` acquires a write lock just to read file metadata. Should be an AtomicU64 tracking WAL position.

### D-08: Two different QuantizedVector types with same name

**Files:** `src/quantization/scalar.rs` vs `src/index/hnsw/native/quantization.rs`

`quantization::scalar::QuantizedVector` has (data, min, max) for per-vector quantization.
`hnsw::native::quantization::QuantizedVector` has only (data) for per-dimension quantization via ScalarQuantizer.

Same name, different semantics, different structs. Confusing.

### D-09: parse_fusion_clause silently swallows invalid parameters

**File:** `src/velesql/parser/conditions.rs` (line ~224)

`val.as_str().parse::<f64>().unwrap_or(0.0)` — invalid fusion parameter values silently become 0.0.

**Impact:** User writes `USING FUSION 'rrf' (k = 'abc')` → k becomes 0.0, no error.

---

## 📝 MINOR — Code Smells

### M-01: Unused validation functions kept "for future"

**File:** `src/velesql/validation.rs`

`contains_similarity()` and `has_not_similarity()` are `#[allow(dead_code)]`.

### M-02: OrderedFloat unreachable!() for NaN handling

**File:** `src/collection/graph/range_index.rs`

The `(false, false)` branch uses `unreachable!()`. While theoretically correct (if both are not NaN, partial_cmp should succeed), it's fragile.

### M-03: Parallel traverser break vs continue inconsistency

**File:** `src/collection/search/query/parallel_traversal/traverser.rs`

`bfs_single` (line ~103) uses `break` when limit reached.
`dfs_single` (line ~211) uses `continue` when limit reached.
BFS stops immediately, DFS keeps popping stack. Different termination semantics.

---

## Phase 2 Findings: Server, WASM, SDK, CI, Integrations

### S-01: No authentication/authorization on HTTP server (CRITICAL)

**File:** `crates/velesdb-server/src/main.rs`

Zero auth middleware. `CorsLayer::permissive()` allows any origin. The TypeScript SDK has an `apiKey` field but the server ignores it entirely. Anyone with network access can read/write/delete all data.

### S-02: Search/query handlers block the async runtime (BUG)

**File:** `crates/velesdb-server/src/handlers/search.rs`, `query.rs`

Most handlers are `#[allow(clippy::unused_async)]` — they don't `await` anything. Only `upsert_points` uses `spawn_blocking`. HNSW search, query execution, and graph traversal are CPU-intensive but run on the async runtime thread. Under load, this will starve other connections.

### S-03: GraphService is disconnected from velesdb-core graph

**File:** `crates/velesdb-server/src/main.rs:64-72`

`GraphService::new()` creates an **in-memory only** graph store separate from velesdb-core's edge store. Two parallel graph systems exist — the REST API graph and the core graph are completely disconnected. Data is lost on restart.

### S-04: No rate limiting on any endpoint (DESIGN)

**File:** `crates/velesdb-server/src/main.rs`

No `tower` rate-limiting middleware. DoS vulnerability. Batch endpoints accept up to 100MB body (`DefaultBodyLimit::max(100 * 1024 * 1024)`).

### W-01: WASM insert_batch ignores storage mode (BUG)

**File:** `crates/velesdb-wasm/src/lib.rs:544-569`

`insert_batch` always pushes to `self.data` (f32 buffer), even when `storage_mode` is SQ8 or Binary. Data stored in wrong format; subsequent searches would compute garbage distances.

```rust
// Bug: always uses Full mode path regardless of storage_mode
self.data.extend_from_slice(&vector);
```

### W-02: WASM hybrid_search silently falls back for non-Full mode (BUG)

**File:** `crates/velesdb-wasm/src/lib.rs:337-339`

If `storage_mode != Full`, `hybrid_search` silently falls back to vector-only search. The text component is completely ignored without any warning to the user.

```rust
if self.storage_mode != StorageMode::Full {
    return self.search(query_vector, k); // Text query silently dropped
}
```

### W-03: No HNSW in WASM — all searches are brute-force O(n) (DESIGN)

**File:** `crates/velesdb-wasm/src/store_search.rs`

WASM VectorStore uses linear scan for ALL searches. No ANN index. For >10K vectors this becomes impractical, but the API makes no distinction from the indexed server-side search.

### T-01: TypeScript SDK `search()` doesn't unwrap server response (BUG)

**File:** `sdks/typescript/src/backends/rest.ts:324-352`

Server returns `{ results: [...] }` but SDK does `return response.data ?? []` without extracting `.results`. Other methods (textSearch, hybridSearch) correctly extract `.results`. Regular `search()` returns a wrapper object typed as `SearchResult[]`.

### T-02: SDK `listCollections` type mismatch with server (BUG)

**File:** `sdks/typescript/src/backends/rest.ts:262-272`

Server returns `{ collections: ["name1", "name2"] }` (string array). SDK declares `request<Collection[]>` and returns `response.data ?? []`. The actual data is `{ collections: [...] }`, not `Collection[]`. Returns wrong structure.

### T-03: SDK `query()` ignores collection parameter (DESIGN)

**File:** `sdks/typescript/src/backends/rest.ts:481-534`

The `collection` parameter is in the function signature but never used in the URL (`POST /query`). The server extracts collection from the VelesQL `FROM` clause. If user passes a different collection name, it's silently ignored.

### CI-01: PR CI is disabled — no pre-merge validation (DESIGN)

**File:** `.github/workflows/ci.yml:22-29`

Pull request CI is commented out. Code goes directly to main/develop without CI validation. Comment says "Validation locale OBLIGATOIRE" but there's no enforcement mechanism.

### CI-02: Security audit never fails CI (DESIGN)

**File:** `.github/workflows/ci.yml:135`

`cargo audit --ignore RUSTSEC-2024-0320 || true` — all audit failures are swallowed. The `|| true` means even critical CVEs won't block deployment.

### CI-03: `cargo deny` not in CI pipeline (DESIGN)

**File:** `.github/workflows/ci.yml`

Quality gates mandate `cargo deny check` but CI only runs `cargo audit`. `cargo deny` catches license violations, banned crates, and duplicate deps. Present in `local-ci.ps1` but not enforced in GitHub Actions.

### CI-04: Python integration tests silently swallowed (DESIGN)

**File:** `.github/workflows/ci.yml:163`

`pytest ... 2>/dev/null || echo "Tests skipped"` — Python test failures are invisible. LangChain tests aren't run at all (only LlamaIndex). Integration regressions go undetected.

### I-01: LangChain `_generate_id()` counter resets per instance (BUG)

**File:** `integrations/langchain/src/langchain_velesdb/vectorstore.py:120,157-161`

`self._next_id = 1` on every new instance. If you create a new `VelesDBVectorStore` pointing to the same collection, IDs collide with existing data, overwriting vectors silently.

### I-02: LlamaIndex `velesql()` missing query validation (BUG)

**File:** `integrations/llamaindex/src/llamaindex_velesdb/vectorstore.py:634-647`

`velesql()` calls `self._collection.query(query_str, params)` without calling `validate_query()`. Every other query method validates input. SQL injection risk through this path.

### I-03: Heavy code duplication across integrations (DESIGN)

**Files:** `integrations/langchain/`, `integrations/llamaindex/`

LangChain and LlamaIndex vectorstore share ~80% identical code (result parsing, `_stable_hash_id`, payload building, batch methods). No shared base package. Bug fixes must be applied twice.

### I-04: GPU Phase 1 scope incomplete — missing Hamming/Jaccard shaders (DESIGN)

**File:** `crates/velesdb-core/src/gpu/gpu_backend.rs`

5 distance metrics exist (Cosine, Euclidean, DotProduct, Hamming, Jaccard) but only Cosine has a real GPU pipeline. Euclidean/DotProduct have dead-code shaders needing wiring. Hamming/Jaccard have no shaders at all.

---

## Summary Table

| ID | Severity | Subsystem | Description |
|----|----------|-----------|-------------|
| C-01 | 🚨 | GPU | batch_euclidean/dot_product are CPU, not GPU |
| C-02 | 🚨 | GPU | GpuTrigramAccelerator has zero GPU code |
| C-03 | 🚨 | GPU | Brute force GPU ignores distance metric |
| C-04 | 🚨 | Fusion | RRF formula is mathematically wrong |
| S-01 | 🚨 | Server | No authentication/authorization |
| B-01 | 🐛 | VelesQL | NaN/Infinity vectors pass validation |
| B-02 | 🐛 | VelesQL | ORDER BY property paths silent no-op |
| B-03 | 🐛 | Fusion | Weighted = Average (no actual weights) |
| B-04 | 🐛 | HNSW | DualPrecision default search isn't dual |
| B-05 | 🐛 | Graph | BFS visited overflow clears visited set |
| B-06 | 🐛 | Quant | cosine_quantized full dequant for norm |
| S-02 | 🐛 | Server | Handlers block async runtime (no spawn_blocking) |
| W-01 | 🐛 | WASM | insert_batch ignores storage mode |
| W-02 | 🐛 | WASM | hybrid_search silently drops text for non-Full |
| T-01 | 🐛 | SDK | search() doesn't unwrap server response |
| T-02 | 🐛 | SDK | listCollections type mismatch |
| I-01 | 🐛 | Integr | _generate_id() counter resets per instance |
| I-02 | 🐛 | Integr | velesql() missing query validation |
| D-01 | ⚠️ | Column | Dual deletion tracking (redundant) |
| D-02 | ⚠️ | HNSW | Layer lock per-iteration contention |
| D-03 | ⚠️ | Graph | CART Node4 dead, leaf splitting absent |
| D-04 | ⚠️ | Query | Over-fetch factor hardcoded |
| D-05 | ⚠️ | Storage | WAL no per-entry CRC |
| D-06 | ⚠️ | Storage | Flush per store (throughput killer) |
| D-07 | ⚠️ | Storage | Write lock for read-only metadata |
| D-08 | ⚠️ | Quant | Two QuantizedVector types, same name |
| D-09 | ⚠️ | VelesQL | Fusion params silently default to 0.0 |
| S-03 | ⚠️ | Server | GraphService disconnected from core graph |
| S-04 | ⚠️ | Server | No rate limiting |
| W-03 | ⚠️ | WASM | No ANN index — brute force O(n) only |
| T-03 | ⚠️ | SDK | query() ignores collection parameter |
| I-03 | ⚠️ | Integr | 80% code duplication LangChain/LlamaIndex |
| I-04 | ⚠️ | GPU | Missing Hamming/Jaccard GPU shaders |
| CI-01 | ⚠️ | CI | PR CI disabled — no pre-merge validation |
| CI-02 | ⚠️ | CI | Security audit never fails CI |
| CI-03 | ⚠️ | CI | cargo deny not in CI pipeline |
| CI-04 | ⚠️ | CI | Python integration tests silently swallowed |
| M-01 | 📝 | VelesQL | Dead validation functions |
| M-02 | 📝 | Graph | unreachable!() in OrderedFloat |
| M-03 | 📝 | Graph | break vs continue in traversal |

---

## Phase 3 Findings: Beginner-Level Architectural Issues

### BEG-01: WASM VectorStore is a full reimplementation, not a binding (CRITICAL ARCH)

**File:** `crates/velesdb-wasm/src/lib.rs:111-129`

WASM VectorStore is a **completely separate vector database** with its own flat `Vec<f32>`/`Vec<u8>` arrays, its own brute-force search, its own graph store, its own agent module. It only uses velesdb-core for `DistanceMetric` enum and VelesQL parser. Two parallel vector DB implementations exist with different bugs and different behavior.

### BEG-02: `storage_mode` is dead code in both LangChain and LlamaIndex (BUG)

**File:** `integrations/langchain/src/langchain_velesdb/vectorstore.py:148-152`

`storage_mode` is validated and stored in `self._storage_mode` but **never passed to `create_collection()`**. Users configuring `storage_mode="sq8"` or `"binary"` always get full f32. Same bug in LlamaIndex.

### BEG-03: `add_texts_bulk` is pure copy-paste of `add_texts` (DESIGN)

**File:** `integrations/langchain/src/langchain_velesdb/vectorstore.py:649-704`

100% identical code to `add_texts` except calling `upsert_bulk` instead of `upsert`. Zero factoring.

### BEG-04: Security validation is theater (DESIGN)

**File:** `integrations/langchain/src/langchain_velesdb/security.py:42-69`

`validate_path` normalizes and rejects `../` but never checks path is inside an allowed directory. `C:\Windows\System32` passes. `validate_query` checks only length and null bytes — zero semantic validation.

### BEG-05: Three parallel BFS/DFS implementations (CRITICAL ARCH)

**Files:** `velesdb-core/collection/search/query/parallel_traversal/`, `velesdb-server/handlers/graph/service.rs`, `velesdb-wasm/src/graph.rs`

Three completely independent graph traversal implementations with different bugs, different behavior, and no shared code. Server version has O(depth²) path cloning.

### BEG-06: 16 clippy allows suppress all quality checks in WASM crate (DESIGN)

**File:** `crates/velesdb-wasm/src/lib.rs:1-16`

`#![allow(clippy::pedantic)]` + `#![allow(clippy::nursery)]` + 14 more specific allows. All quality analysis disabled — equivalent to suppressing all compiler warnings.

### BEG-07: SDK `init()` TOCTOU race condition (BUG)

**File:** `sdks/typescript/src/backends/rest.ts:59-77`

`_initialized` flag is not guarded by a mutex/promise lock. If two concurrent async calls trigger before init completes, both execute health check simultaneously and the flag can be set while another caller is mid-check.

---

## Summary Table

| ID | Severity | Subsystem | Description |
|----|----------|-----------|-------------|
| C-01 | 🚨 | GPU | batch_euclidean/dot_product are CPU, not GPU |
| C-02 | 🚨 | GPU | GpuTrigramAccelerator has zero GPU code |
| C-03 | 🚨 | GPU | Brute force GPU ignores distance metric |
| C-04 | 🚨 | Fusion | RRF formula is mathematically wrong |
| S-01 | 🚨 | Server | No authentication/authorization |
| BEG-01 | 🚨 | WASM | Full reimplementation, not a binding |
| BEG-05 | 🚨 | Arch | Three parallel BFS/DFS implementations |
| B-01 | 🐛 | VelesQL | NaN/Infinity vectors pass validation |
| B-02 | 🐛 | VelesQL | ORDER BY property paths silent no-op |
| B-03 | 🐛 | Fusion | Weighted = Average (no actual weights) |
| B-04 | 🐛 | HNSW | DualPrecision default search isn't dual |
| B-05 | 🐛 | Graph | BFS visited overflow clears visited set |
| B-06 | 🐛 | Quant | cosine_quantized full dequant for norm |
| S-02 | 🐛 | Server | Handlers block async runtime (no spawn_blocking) |
| W-01 | 🐛 | WASM | insert_batch ignores storage mode |
| W-02 | 🐛 | WASM | hybrid_search silently drops text for non-Full |
| T-01 | 🐛 | SDK | search() doesn't unwrap server response |
| T-02 | 🐛 | SDK | listCollections type mismatch |
| I-01 | 🐛 | Integr | _generate_id() counter resets per instance |
| I-02 | 🐛 | Integr | velesql() missing query validation |
| BEG-02 | 🐛 | Integr | storage_mode dead code (never passed) |
| BEG-07 | 🐛 | SDK | init() TOCTOU race condition |
| D-01 | ⚠️ | Column | Dual deletion tracking (redundant) |
| D-02 | ⚠️ | HNSW | Layer lock per-iteration contention |
| D-03 | ⚠️ | Graph | CART Node4 dead, leaf splitting absent |
| D-04 | ⚠️ | Query | Over-fetch factor hardcoded |
| D-05 | ⚠️ | Storage | WAL no per-entry CRC |
| D-06 | ⚠️ | Storage | Flush per store (throughput killer) |
| D-07 | ⚠️ | Storage | Write lock for read-only metadata |
| D-08 | ⚠️ | Quant | Two QuantizedVector types, same name |
| D-09 | ⚠️ | VelesQL | Fusion params silently default to 0.0 |
| S-03 | ⚠️ | Server | GraphService disconnected from core graph |
| S-04 | ⚠️ | Server | No rate limiting |
| W-03 | ⚠️ | WASM | No ANN index — brute force O(n) only |
| T-03 | ⚠️ | SDK | query() ignores collection parameter |
| I-03 | ⚠️ | Integr | 80% code duplication LangChain/LlamaIndex |
| I-04 | ⚠️ | GPU | Missing Hamming/Jaccard GPU shaders |
| CI-01 | ⚠️ | CI | PR CI disabled — no pre-merge validation |
| CI-02 | ⚠️ | CI | Security audit never fails CI |
| CI-03 | ⚠️ | CI | cargo deny not in CI pipeline |
| CI-04 | ⚠️ | CI | Python integration tests silently swallowed |
| BEG-03 | ⚠️ | Integr | add_texts_bulk pure copy-paste |
| BEG-04 | ⚠️ | Integr | Security validation is theater |
| BEG-06 | ⚠️ | WASM | 16 clippy allows suppress all quality |
| M-01 | 📝 | VelesQL | Dead validation functions |
| M-02 | 📝 | Graph | unreachable!() in OrderedFloat |
| M-03 | 📝 | Graph | break vs continue in traversal |

**Total: 47 findings** (7 critical, 14 bugs, 23 design, 3 minor)
