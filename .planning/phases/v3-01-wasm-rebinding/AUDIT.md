# WASM Crate Audit — Core Mapping & Refactoring Opportunities

> **Date**: 2026-02-09
> **Scope**: `crates/velesdb-wasm/src/` vs `crates/velesdb-core/src/`
> **Context**: WASM imports core with `default-features = false` (no `persistence` feature).

---

## 1. Core Modules Available to WASM (no `persistence`)

| Core Module | Key Exports | Available to WASM |
|---|---|---|
| `distance` | `DistanceMetric` | ✅ |
| `error` | `Error`, `Result` | ✅ |
| `filter` | `Filter`, `Condition` | ✅ |
| `fusion` | `FusionStrategy`, `FusionError` | ✅ |
| `point` | `Point`, `SearchResult` | ✅ |
| `quantization` | `StorageMode`, `QuantizedVector`, `BinaryQuantizedVector`, distance fns | ✅ |
| `simd_native` | x86 AVX2/AVX-512, NEON kernels | ✅ (but **wrong ISA** for WASM) |
| `simd_dispatch` | `SimdLevel`, feature detection | ✅ (but x86/ARM only) |
| `velesql` | `Parser`, `Query`, AST types | ✅ |
| `config` | `VelesConfig`, `HnswConfig`, etc. | ✅ |
| `half_precision` | f16/bf16 conversion | ✅ |
| `alloc_guard` | Memory allocation guard | ✅ |
| `cache` | LRU cache | ✅ |
| `compression` | Compression utilities | ✅ |
| `metrics` | IR metrics (recall, precision, MRR, nDCG) | ✅ |
| `sync` | Synchronization primitives | ✅ |
| `vector_ref` | Zero-copy vector references | ✅ |

### NOT available (behind `persistence` feature)

| Core Module | Key Exports | Reason |
|---|---|---|
| `collection` | `Collection`, `GraphNode`, `GraphEdge`, `GraphSchema`, traversal | Uses `memmap2`, `rayon`, `tokio` |
| `agent` | Agent memory | Depends on `collection` |
| `storage` | `MmapStorage` | Uses `memmap2` |
| `index` | `HnswIndex`, `HnswParams` | Uses `rayon` |
| `column_store` | `ColumnStore`, `TypedColumn` | Uses `rayon` |
| `guardrails` | Query guard-rails | Depends on `collection` |
| `Database` | Top-level database | Depends on everything above |

---

## 2. VectorStore API → Core Mapping

### 2.1 `VectorStore` struct (`lib.rs`)

| WASM API | Core Equivalent | Verdict |
|---|---|---|
| `VectorStore` struct | `Collection` (persistence) | ❌ **No core equivalent** — WASM must own this |
| `StorageMode` enum | `quantization::StorageMode` ✅ | ⚠️ **DUPLICATED** — WASM redefines its own enum |
| `DistanceMetric` | `distance::DistanceMetric` ✅ | ✅ **Already delegated** via re-export |
| `SearchResult` struct | `point::SearchResult` ✅ | ⚠️ **DUPLICATED** — WASM defines its own simpler version |
| `QueryResult` struct | No core equivalent | ✅ WASM-only (justified for JS interop) |

### 2.2 Store sub-modules

| WASM Module | Role | Core Equivalent | Verdict |
|---|---|---|---|
| `store_new.rs` | Constructor, parsing | `DistanceMetric::from()` | ⚠️ Partial reimplementation of parsing |
| `store_insert.rs` | Vector insertion + quantization | None (core uses `Collection::upsert`) | ✅ WASM-only (justified) |
| `store_search.rs` | k-NN, similarity, hybrid, batch | None (core uses `Collection::search`) | ✅ WASM-only (justified) |
| `store_get.rs` | Get by ID | None (core uses `Collection::get`) | ✅ WASM-only (justified) |

### 2.3 Distance / SIMD

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `simd.rs` (uses `wide` crate for WASM SIMD128) | `simd_native/` (AVX2, AVX-512, NEON) | ❌ **Different ISA** | ✅ **Justified reimplementation** — WASM needs SIMD128, core targets x86/ARM |

> **Note**: The `wide` crate provides portable SIMD that compiles to WASM SIMD128.
> Core's `simd_native` uses platform-specific intrinsics (`_mm256_*`, `vfmaq_f32`, etc.)
> that don't exist in the WASM target. This reimplementation is **necessary**.

### 2.4 Filtering

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `filter.rs` — `matches_filter(payload, filter_json)` | `filter::Filter::matches(payload)` ✅ | ⚠️ **Different interface** | 🔶 **Refactoring candidate** |

**Details**:
- WASM `filter.rs` operates on raw `serde_json::Value` for both payload AND filter definition
- Core `filter::Filter` uses typed `Condition` enum with builder pattern
- WASM filter accepts JSON like `{"field": "name", "op": "eq", "value": "hello"}`
- Core filter uses `Condition::eq("name", "hello")`

**Recommendation**: Create a `Condition::from_json(value: &serde_json::Value) -> Result<Condition>` in core, then WASM can:
1. Parse JS filter JSON → core `Condition`
2. Call `condition.matches(payload)` from core

This would **eliminate ~160 lines** of duplicated matching logic in WASM.

### 2.5 Fusion

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `fusion.rs` — `fuse_results(results, strategy)` | `fusion::FusionStrategy::fuse(results)` ✅ | ⚠️ **Different interface** | 🔶 **Refactoring candidate** |

**Details**:
- WASM: `fuse_results(results: Vec<Vec<(String, f32)>>, strategy: &str, rrf_k: Option<u32>)`
- Core: `FusionStrategy::fuse(results: Vec<Vec<(u64, f32)>>)` → `Result<Vec<(u64, f32)>>`
- WASM uses `String` IDs; core uses `u64` IDs
- WASM parses strategy from string; core uses typed enum

**Recommendation**: WASM can convert `String → u64` IDs at the boundary, then delegate to `FusionStrategy::fuse()`. This would **eliminate ~90 lines** of duplicated fusion logic.

### 2.6 Quantization

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `quantization.rs` — SQ8 + Binary helpers | `quantization::QuantizedVector`, `BinaryQuantizedVector` ✅ | ⚠️ **Different API shape** | 🔶 **Refactoring candidate** |

**Details**:
- WASM: Loose functions (`compute_sq8_params`, `quantize_sq8`, `dequantize_sq8`, `pack_binary`, `unpack_binary`)
- Core: Struct-based API (`QuantizedVector::from_f32()`, `BinaryQuantizedVector::from_f32()`)
- The core API is richer (serialization, memory size, etc.)

**Recommendation**: Replace WASM functions with calls to core's `QuantizedVector` and `BinaryQuantizedVector`. The structs are available without `persistence`. This would **eliminate ~100 lines**.

### 2.7 Parsing Utilities

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `parsing.rs` — `parse_metric()`, `parse_storage_mode()`, `validate_dimension()` | `DistanceMetric` has `from_str` | ⚠️ Partial overlap | 🔶 Minor refactoring candidate |

### 2.8 VelesQL Parser

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `velesql.rs` — `VelesQL::parse()`, `ParsedQuery` | `velesql::Parser::parse()` ✅ | ✅ **Already delegated** | ✅ Good — thin WASM binding over core |

### 2.9 Persistence & Serialization

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `persistence.rs` — IndexedDB for VectorStore | `storage::MmapStorage` ❌ (persistence) | ❌ | ✅ **Justified** — browser needs IndexedDB |
| `serialization.rs` — binary export/import | None | ❌ | ✅ **Justified** — WASM-specific format |
| `text_search.rs` — substring matching | Core uses BM25 (persistence) | ❌ | ✅ **Justified** — lightweight alternative |

---

## 3. GraphStore + Agent → Core Mapping

### 3.1 Graph Module

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `graph.rs` → `GraphNode` | `collection::GraphNode` ❌ (persistence) | ❌ | ✅ **Justified reimplementation** |
| `graph.rs` → `GraphEdge` | `collection::GraphEdge` ❌ (persistence) | ❌ | ✅ **Justified reimplementation** |
| `graph.rs` → `GraphStore` | `collection::EdgeStore` ❌ (persistence) | ❌ | ✅ **Justified reimplementation** |
| `graph.rs` → BFS/DFS traversal | `collection::graph::bfs_stream` ❌ (persistence) | ❌ | ✅ **Justified reimplementation** |

> **Root cause**: Core's entire graph subsystem lives inside `collection/` which requires
> `memmap2` + `rayon` + `tokio` — all incompatible with WASM target.

### 3.2 Graph Persistence & Workers

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `graph_persistence.rs` — IndexedDB for GraphStore | None | ❌ | ✅ **WASM-only** (browser storage) |
| `graph_worker.rs` — Web Worker offloading | None | ❌ | ✅ **WASM-only** (browser threading) |

### 3.3 Agent / Semantic Memory

| WASM Module | Core Module | Can Delegate? | Verdict |
|---|---|---|---|
| `agent.rs` → `SemanticMemory` | `agent` ❌ (persistence) | ❌ | ✅ **Justified reimplementation** |

---

## 4. Full Duplication Catalog

### 🔴 Modules fully reimplemented (should delegate to core)

| WASM Module | Lines | Core Module | Savings Estimate |
|---|---|---|---|
| `filter.rs` | ~264 | `filter::Condition::matches()` | ~160 lines |
| `fusion.rs` | ~145 | `fusion::FusionStrategy::fuse()` | ~90 lines |
| `quantization.rs` | ~148 | `quantization::{QuantizedVector, BinaryQuantizedVector}` | ~100 lines |
| `StorageMode` enum in `lib.rs` | ~10 | `quantization::StorageMode` | ~10 lines |
| **Total** | | | **~360 lines** |

### 🟡 Modules partially reimplemented (minor delegation possible)

| WASM Module | Lines | Overlap | Notes |
|---|---|---|---|
| `parsing.rs` | ~155 | `DistanceMetric` parsing | Could use core's `FromStr` impl |
| `store_new.rs` | ~50 | Metric/mode parsing | Already uses core `DistanceMetric` |

### 🟢 Justified WASM-only modules (no core equivalent possible)

| WASM Module | Lines | Reason |
|---|---|---|
| `lib.rs` (VectorStore) | ~626 | Core's `Collection` needs filesystem |
| `store_insert.rs` | ~100 | In-memory vector storage |
| `store_search.rs` | ~300 | In-memory brute-force search |
| `store_get.rs` | ~50 | In-memory get by ID |
| `simd.rs` | ~274 | WASM SIMD128 via `wide` crate (different ISA) |
| `vector_ops.rs` | ~260 | Score computation for WASM VectorStore |
| `text_search.rs` | ~115 | Lightweight substring search (no BM25) |
| `serialization.rs` | ~200 | Binary export/import for IndexedDB |
| `persistence.rs` | ~126 | IndexedDB persistence |
| `graph.rs` | ~550 | Graph data structures (core needs persistence) |
| `graph_persistence.rs` | ~321 | IndexedDB multi-graph persistence |
| `graph_worker.rs` | ~307 | Web Worker offloading |
| `agent.rs` | ~176 | SemanticMemory (core needs persistence) |
| `velesql.rs` | ~478 | Thin binding — already delegates to core |

---

## 5. Recommended Refactoring Actions

### Priority 1: Eliminate `filter.rs` duplication

**Effort**: Medium | **Impact**: High (core filter has LIKE, ILIKE, IN, IsNull — WASM lacks these)

1. Add `Condition::from_json_value(&serde_json::Value) -> Result<Condition>` to core's `filter` module
2. Replace WASM `filter.rs` with:
   ```rust
   use velesdb_core::filter::Condition;
   pub fn matches_filter(payload: &Value, filter: &Value) -> bool {
       Condition::from_json_value(filter)
           .map(|c| c.matches(payload))
           .unwrap_or(false)
   }
   ```
3. WASM gains LIKE, ILIKE, IN, IsNull support for free

### Priority 2: Eliminate `fusion.rs` duplication

**Effort**: Low | **Impact**: Medium

1. Replace WASM `fusion.rs` with wrapper calling `FusionStrategy::fuse()`
2. Convert String IDs → u64 at boundary
3. Parse strategy string → `FusionStrategy` enum

### Priority 3: Eliminate `quantization.rs` duplication

**Effort**: Low | **Impact**: Medium

1. Replace `quantize_sq8()` / `dequantize_sq8()` with `QuantizedVector::from_f32()` / `.to_f32()`
2. Replace `pack_binary()` / `unpack_binary()` with `BinaryQuantizedVector::from_f32()` / `.get_bits()`

### Priority 4: Unify `StorageMode` enum

**Effort**: Trivial | **Impact**: Low (consistency)

1. Re-export `velesdb_core::StorageMode` with `#[wasm_bindgen]` wrapper
2. Remove duplicate enum definition

---

## 6. Architectural Consideration: Extract Graph to Shared Crate

The biggest duplication is the graph subsystem (~550 lines in WASM) because core's `GraphNode`/`GraphEdge`/traversal
lives inside `collection/` which depends on persistence.

**Future option**: Extract a `velesdb-graph-core` crate with:
- `GraphNode`, `GraphEdge` structs (no persistence deps)
- BFS/DFS iterators (pure algorithm, no I/O)
- Both `velesdb-core` and `velesdb-wasm` depend on it

This would eliminate the largest single source of duplication but requires a new crate.

---

## 7. Summary

| Category | Lines in WASM | Can Delegate | Savings |
|---|---|---|---|
| 🔴 Full duplicates | ~567 | ✅ Yes | ~360 lines |
| 🟡 Partial overlap | ~205 | Partial | ~30 lines |
| 🟢 Justified WASM-only | ~3,883 | ❌ No | 0 |
| **Total WASM src** | **~4,655** | | **~390 lines (~8%)** |

> **Bottom line**: ~8% of WASM code is unnecessarily duplicated from core.
> The remaining 92% is justified by ISA differences (SIMD), platform differences
> (IndexedDB vs filesystem, Web Workers), or feature-gating (`persistence`).
