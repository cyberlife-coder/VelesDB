---
phase: v3-01
plan: 01
name: Audit WASM → Core Mapping
status: complete
completed: 2026-02-09
---

# Plan 01 Summary: Audit WASM → Core Mapping

## What Was Done

All 3 tasks completed:

### Task 1: Map VectorStore API to Core Equivalents ✅
- Every `#[wasm_bindgen]` method on `VectorStore` mapped to core equivalent or marked as GAP
- `DistanceMetric` already delegated to core ✅
- `StorageMode` identified as **duplicate** of `velesdb_core::quantization::StorageMode`
- `SearchResult` identified as **duplicate** of `velesdb_core::point::SearchResult`
- ECO-06 (insert_batch ignores storage_mode) documented
- ECO-07 (hybrid_search drops text for non-Full) documented

### Task 2: Map GraphStore + Agent API to Core Equivalents ✅
- All graph types (`GraphNode`, `GraphEdge`, `GraphStore`) are reimplemented — core equivalents locked behind `persistence` feature
- `SemanticMemory` (agent.rs) is reimplemented — core `agent` module locked behind `persistence`
- `graph_persistence.rs` (IndexedDB) and `graph_worker.rs` (Web Worker) confirmed as WASM-specific (keep)
- **Key GAP:** Core's `EdgeStore`, `GraphNode`, `GraphEdge`, BFS/DFS all require persistence feature → need extraction

### Task 3: Catalog Reimplemented Modules + Core Availability ✅

| WASM Module | Lines | Classification | Core Available? |
|---|---|---|---|
| `simd.rs` | 274 | DELETE | ❌ Wrong ISA — but dead code (unused), safe to delete |
| `filter.rs` | 264 | DELETE | ✅ `filter::Condition::matches()` |
| `fusion.rs` | 145 | DELETE | ✅ `fusion::FusionStrategy::fuse()` |
| `quantization.rs` | 148 | DELETE | ✅ `quantization::{QuantizedVector, BinaryQuantizedVector}` |
| `vector_ops.rs` | 260 | REFACTOR | Partial — scoring loop is WASM-specific |
| `text_search.rs` | 115 | KEEP | ❌ Core text search behind persistence |
| `velesql.rs` | 478 | KEEP | ✅ Already delegates to core parser |
| `serialization.rs` | ~200 | KEEP | ❌ WASM-specific binary format |
| `persistence.rs` | ~126 | KEEP | ❌ WASM-specific IndexedDB |
| `graph.rs` | ~550 | REFACTOR | ❌ Behind persistence → need extraction |
| `graph_persistence.rs` | ~321 | KEEP | ❌ WASM-specific IndexedDB |
| `graph_worker.rs` | ~307 | KEEP | ❌ WASM-specific Web Worker |
| `agent.rs` | ~176 | KEEP | ❌ Behind persistence |

**`wide` crate removal confirmed safe** — `simd.rs` is dead code, no callers outside that file.

## Artifacts

- `.planning/phases/v3-01-wasm-rebinding/AUDIT.md` — Complete mapping document (279 lines)

## Key Findings for Subsequent Plans

1. **~360 lines deletable** via delegation to core (filter, fusion, quantization, StorageMode)
2. **Graph extraction needed** (Plan 02) — core's graph types behind persistence feature
3. **`json_to_condition()` needed** (Plan 02) — WASM filter uses JSON format, core uses typed `Condition`
4. **SIMD is dead code** — can delete immediately in Plan 03
5. **Agent is thin wrapper** — acceptable to keep after VectorStore rebinding (Plan 05 decision)

## Success Criteria Met

- [x] Every WASM public method mapped to core equivalent or marked as GAP
- [x] Persistence-gated gaps identified (EdgeStore, agent, text search)
- [x] DELETE/KEEP/REFACTOR classification for all source files
- [x] `wide` crate removal confirmed safe
- [x] Mapping document committed
