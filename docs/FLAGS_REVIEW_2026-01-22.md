# Flags Review - 2026-01-22

## Summary

**6 Bugs Fixed** | **23 Flags Categorized**

---

## Bugs Fixed (This PR)

| Bug | Location | Fix |
|-----|----------|-----|
| BUG-1/3 | query/mod.rs:92 | Increased over-fetch from 4x to 10x for threshold accuracy |
| BUG-2 | query/mod.rs:252 | Recompute similarity with query vector, not NEAR scores |
| BUG-4 | query/mod.rs:107,143 | Handle null payload with filter.matches(null) |
| BUG-5 | query/mod.rs:95,142 | Validate field name, error if not "vector" |
| BUG-6 | graph.rs:42-97 | Replace expect() with Result for graceful error handling |

---

## Flags Categorization

### ✅ Already Fixed (Prior Work)

| Flag | Location | Status |
|------|----------|--------|
| Python BFS unwrap_or(0) | graph_store.rs:234 | Fixed with filter_map |
| RoaringBitmap u32 limit | property_index.rs:61 | Has try_from bounds check |
| REPL browse underflow | repl_commands.rs:238 | Fixed with .max(1) |
| GPU tests serial | gpu_backend_tests.rs | Has #[serial(gpu)] |

### 📋 Design Decisions (Documented)

| Flag | Location | Rationale |
|------|----------|-----------|
| GraphService isolated stores | graph.rs:24-88 | Multi-tenant design - each collection has isolated graph |
| Index persistence graceful | lifecycle.rs:231-269 | Logs warning, starts fresh - avoids startup failure |
| PropertyIndex versioning | lifecycle.rs | Future improvement, binary format stable for now |
| Asymmetric OR handling | extraction.rs:152-162 | Intentional SQL semantics - OR requires both sides |
| ORDER BY double-inversion | ordering.rs:139-154 | Metric-aware semantics for distance vs similarity |
| Adaptive shard log2 | edge_concurrent.rs:92-107 | Integer math avoids float imprecision |
| Duration overflow cap | metrics.rs:43-66 | Caps at u64::MAX for safety |
| WasmBackend stubs | wasm.ts:416-443 | Explicitly documented, throws clear errors |

### ⚠️ Known Limitations (Acceptable)

| Flag | Location | Note |
|------|----------|------|
| similarity() top-K window | query/mod.rs:90-93 | ANN limitation, 10x overfetch mitigates |
| similarity(field) single | query/mod.rs:95-102 | Only "vector" field supported |
| ORDER BY multi-similarity | ordering.rs:89-95 | First similarity() pre-computed only |
| ConcurrentEdgeStore lock | edge_concurrent.rs:204-226 | Read during write is intentional for atomicity |

### 🔮 Future Improvements (Backlog)

| Flag | Location | EPIC |
|------|----------|------|
| Multi-vector fields | - | Future EPIC |
| Full threshold scan | - | For exact semantics |
| Index versioning | lifecycle.rs | Schema evolution |

---

## Validation

```bash
cargo test --workspace        # 1400+ tests pass
cargo clippy -- -D warnings   # Clean
cargo deny check              # Security OK
```
