# Phase 3: Architecture Extraction & Graph Safety - Research

**Researched:** 2026-02-06
**Domain:** Rust module extraction, HNSW lock-order safety, concurrent resize validation
**Confidence:** HIGH

<user_constraints>
## User Constraints (from CONTEXT.md)

### Locked Decisions
## Implementation Decisions

### Decoupage modules
- Apply a strict 500-line rule with rare, explicitly justified temporary exceptions.
- `simd_native.rs` extraction strategy is delegated to Claude, with expected outcome: coherent, maintainable boundaries and low regression risk.
- Parser extraction should use a hybrid split: clause-oriented modules plus shared cross-cutting validation modules.
- Migration sequencing is delegated to Claude (incremental vs chunked), with requirement to preserve stability and testability.

### Conventions de nommage
- SIMD naming should follow a stable hybrid readability model (ISA visibility + responsibility clarity).
- Naming decisions for new submodules and concurrent HNSW files are delegated to Claude, with priority on long-term readability and consistency.
- Public API compatibility must remain stable during extraction (no breaking API behavior).

### Politique anti-duplication
- Prioritize deduplication of serialization/deserialization patterns first.
- Dedup aggressiveness and merge-vs-separate decisions are delegated to Claude, constrained by readability and regression safety.
- Acceptance guardrail for dedup is mandatory: both targeted tests and call-site readability must remain strong.

### Securite HNSW observable
- Thread-safety invariant set is delegated to Claude, but it must be explicit, auditable, and testable.
- Concurrent test coverage must include both families: (1) parallel insert/search/delete and (2) resize + snapshot consistency.
- Observability minimum is locked: essential counters for contention, failed/retried operations, and detected corruption/invariant incidents.
- Debug/release policy is locked to high parity: keep as many safety checks in release as feasible, only relaxing checks with explicit cost/benefit justification.

### Claude's Discretion
- Exact submodule boundary map for SIMD, HNSW graph internals, and parser internals.
- Final naming taxonomy for files/modules where not explicitly locked.
- Refactor sequencing strategy per subsystem.
- Concrete invariant list and instrumentation details (counter names, hook locations) as long as the locked outcomes above are satisfied.

### Deferred Ideas (OUT OF SCOPE)
## Deferred Ideas

None - discussion stayed within Phase 3 scope.
</user_constraints>

## Summary

Phase 3 planning should be treated as a structural safety phase with zero feature drift: break oversized modules into stable internal boundaries, remove high-value duplication first (serialization/deserialization), and make HNSW concurrency rules executable through runtime checks and counters.

The current codebase already shows the target direction: `index/hnsw/index/` is split by concern (`constructors.rs`, `search.rs`, `batch.rs`), parser logic is already partially extracted (`match_parser.rs`, `conditions.rs`, `values.rs`), and deadlock tests exist for native HNSW. The missing part is consistent extraction for the three oversized files (`simd_native.rs` 2406 lines, `graph.rs` 640 lines, `select.rs` 828 lines), plus auditable lock-order invariants with release-parity observability.

**Primary recommendation:** use an incremental facade-first extraction (keep `simd_native`, `graph`, `select` public entrypoints stable) while introducing internal submodules, lock-order guards, and concurrency counters in parallel with focused regression tests.

### Phased Implementation Recommendation (maps to 2-3 plans)

1. **Plan A - Extraction and naming baseline**: split SIMD + parser + graph into submodules behind stable facades; enforce <=500 line objective per new source file.
2. **Plan B - HNSW graph safety and observability**: implement explicit lock invariants, runtime checker, and counters with high debug/release parity.
3. **Plan C - Concurrency verification and dedup closure**: add two required concurrency test families and complete serde pattern dedup across HNSW constructors/save-load.

## Standard Stack

The established libraries/tools for this phase domain:

### Core
| Library | Version | Purpose | Why Standard |
|---------|---------|---------|--------------|
| Rust toolchain | 1.83 | Baseline compile/lint/test behavior | Workspace pinned in `Cargo.toml` and policy docs |
| `parking_lot` | 0.12 | Fast `RwLock` for HNSW graph and storage | Already used in graph/storage hot paths |
| `pest` + `pest_derive` | 2.7 | SQL parser grammar and parse tree handling | Existing parser foundation; extraction must preserve it |
| `bincode` | 1.3 | HNSW metadata/mappings persistence | Existing save/load format in both index constructor paths |
| `memmap2` | 0.9 | Backing store and guard epoch model | Required for resize + snapshot consistency testing |

### Supporting
| Library | Version | Purpose | When to Use |
|---------|---------|---------|-------------|
| `rustc-hash` | 2.0 | `FxHashSet` for search-layer visited set | Preserve HNSW search performance behavior |
| `rayon` | 1.10 | Parallel batch insert/search operations | Concurrency stress paths and parallel test coverage |
| `loom` | 0.7 (optional feature) | Model-check lock/epoch concurrency contracts | For lock-order and epoch validation tests |
| `tempfile` | 3.14 | Resize/snapshot integration test fixtures | File-backed storage race scenario tests |

### Alternatives Considered
| Instead of | Could Use | Tradeoff |
|------------|-----------|----------|
| `parking_lot::RwLock` graph/storage locking | `std::sync::RwLock` | Higher overhead and behavior drift from existing code |
| Shared HNSW serde helper | Keep duplicated save/load in each index impl | Faster short-term edits, higher long-term drift and bug risk |
| Clause+validation parser split | Monolithic clause-only split | Less cross-module reuse and repeated validation logic |

**Installation:**
```bash
# Existing workspace stack, no new dependency required
cargo test -p velesdb-core
```

## Architecture Patterns

### Recommended Project Structure
```text
crates/velesdb-core/src/
├── simd_native/
│   ├── mod.rs                    # stable public facade (`crate::simd_native::*`)
│   ├── dispatch.rs               # simd level detection + public dispatch
│   ├── tail_unroll.rs            # remainder/tail helpers and macros
│   ├── prefetch.rs               # prefetch distance + prefetch helpers
│   ├── x86_avx512.rs             # avx512 dot/l2/cos + binary metrics
│   ├── x86_avx2.rs               # avx2 dot/l2/cos + binary metrics
│   ├── neon.rs                   # aarch64 NEON kernels
│   └── scalar.rs                 # scalar fallbacks + fast rsqrt helpers
├── index/hnsw/native/graph/
│   ├── mod.rs                    # `NativeHnsw` struct + stable API methods
│   ├── insert.rs                 # insert flow, layer growth, entry-point updates
│   ├── search.rs                 # search/search_multi_entry/search_layer logic
│   ├── neighbors.rs              # select_neighbors + bidirectional/pruning paths
│   ├── locking.rs                # lock rank checker + lock helper wrappers
│   └── safety_counters.rs        # contention/retry/invariant counters
└── velesql/parser/select/
    ├── mod.rs                    # parse_query/parse_select_stmt facade
    ├── clause_compound.rs        # query/set operator parsing
    ├── clause_projection.rs      # select list, columns, aggregates
    ├── clause_from_join.rs       # from/join/column_ref parsing
    ├── clause_group_order.rs     # group by/having/order by parsing
    ├── clause_limit_with.rs      # limit/offset/with/fusion wiring
    └── validation.rs             # cross-cutting aggregate/op/identifier checks
```

### Pattern 1: Facade-First Extraction (No API Break)
**What:** Convert large file to directory module with `mod.rs` facade preserving public function names and paths.
**When to use:** `simd_native.rs`, `index/hnsw/native/graph.rs`, `velesql/parser/select.rs` extraction.
**Example:**
```rust
// Source: crates/velesdb-core/src/index/hnsw/index/mod.rs
mod constructors;
mod search;
mod batch;

pub struct HnswIndex { /* unchanged public shape */ }
```

### Pattern 2: Hybrid Parser Split (Clause + Shared Validation)
**What:** Keep clause parsers isolated, centralize repeated validation/parsing rules (operators, aggregate constraints, identifier quoting).
**When to use:** `select.rs` decomposition; avoid duplicating compare-op and aggregate wildcard rules.
**Example:**
```rust
// Source: crates/velesdb-core/src/velesql/parser/select.rs:414
if matches!(arg, AggregateArg::Wildcard) && !matches!(agg_type, AggregateType::Count) {
    return Err(ParseError::syntax(/* ... */));
}
```

### Pattern 3: Explicit Lock-Rank Invariant with Runtime Checker
**What:** Encode lock order as rank (`vectors=10`, `layers=20`, `neighbors=30`), assert monotonic acquisition per thread, increment violation counters.
**When to use:** Every graph path that acquires more than one lock; especially `insert`, `search_layer`, and `add_bidirectional_connection`.
**Example:**
```rust
// Source: crates/velesdb-core/src/index/hnsw/native/graph.rs:585
// Lock order invariant: vectors -> layers -> neighbors
// Runtime checker should enforce this before lock acquisition.
```

### Pattern 4: Observable Safety with Release Parity
**What:** Keep lightweight safety instrumentation in all builds (atomics + lock-rank checks), reserve only expensive deep graph scans for debug/test.
**When to use:** BUG-04 implementation with locked requirement for debug/release parity.
**Example counters and hook points:**
- `hnsw_lock_contention_total` (increment on failed `try_read`/`try_write` before blocking path)
- `hnsw_operation_retry_total` (increment on retry loops or transient failures)
- `hnsw_invariant_violation_total` (increment before panic/error on rank/order violations)
- `hnsw_corruption_detected_total` (increment when adjacency invariants fail)

### Anti-Patterns to Avoid
- **Chunked mega-move without facade:** breaks compile graph and masks regression source.
- **Debug-only lock checks:** violates high-parity constraint and hides production races.
- **Parser split by syntax only:** duplicates validation logic and increases drift risk.
- **Concurrent tests using sleep-based success criteria:** flaky and non-diagnostic.

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| HNSW save/load plumbing | Two separate bespoke serde implementations | Shared internal HNSW persistence helper reused by `native_index.rs` and `index/constructors.rs` | Existing duplication already exists and is high-risk for drift |
| Concurrency model testing | Ad-hoc random threaded loops only | Existing loom + deterministic thread tests (`storage/loom_tests.rs`, `native/tests.rs`) | Better race/interleaving coverage and reproducibility |
| Lock-order tracking state | Global mutable checker with coarse lock | Thread-local rank stack + atomic counters | Lower overhead and easier per-thread diagnostics |
| Parser rule validations | Repeated inline checks in each clause parser | Shared `validation.rs` functions | Keeps hybrid split readable and consistent |

**Key insight:** this phase succeeds by codifying invariants once and reusing them everywhere, not by duplicating "small" helper logic in each extracted module.

## Common Pitfalls

### Pitfall 1: Extraction that silently changes visibility/contracts
**What goes wrong:** moved functions become private or module paths change for call sites/tests.
**Why it happens:** direct file splitting without facade/re-export plan.
**How to avoid:** keep `mod.rs` stable public surface first, then move internals behind `pub(crate)` exports.
**Warning signs:** compile errors in unrelated modules after first extraction commit.

### Pitfall 2: Reintroducing lock-order inversion via helper refactors
**What goes wrong:** helper acquires `layers` then calls code that acquires `vectors`.
**Why it happens:** lock ordering documented but not enforced at call boundaries.
**How to avoid:** centralize lock helpers in `locking.rs` with rank assertions and counter hooks.
**Warning signs:** intermittent hangs in mixed insert/search stress tests.

### Pitfall 3: Over-dedup harming readability
**What goes wrong:** generic helper API becomes harder to read than duplicated straightforward call-sites.
**Why it happens:** dedup is optimized for line count instead of call-site clarity.
**How to avoid:** dedup only repeated serde and strict boilerplate first; require test + call-site readability checks.
**Warning signs:** helper signatures with many bool/enum knobs and unclear names.

### Pitfall 4: Concurrency tests that do not assert invariants
**What goes wrong:** test only checks "no panic" and misses data corruption or stale guard behavior.
**Why it happens:** stress tests lack post-conditions.
**How to avoid:** add explicit assertions on counts, sortedness, guard epoch validity, and snapshot consistency.
**Warning signs:** passing tests with no deterministic state assertions.

## Code Examples

Verified patterns from current codebase:

### Existing lock-order intent to preserve and enforce
```rust
// Source: crates/velesdb-core/src/index/hnsw/native/graph.rs:587
// Global lock order: vectors -> layers -> neighbors
// Keep this as executable invariant via rank checker + counters.
```

### Existing parser extraction precedent (facade remains in parser/mod.rs)
```rust
// Source: crates/velesdb-core/src/velesql/parser/mod.rs
mod conditions;
mod match_parser;
mod select;
mod values;
```

### Existing serialization duplication target for QUAL-02
```rust
// Source: crates/velesdb-core/src/index/hnsw/native_index.rs and
//         crates/velesdb-core/src/index/hnsw/index/constructors.rs
bincode::serialize_into(writer, &(id_to_idx, idx_to_id, next_idx))?;
bincode::serialize_into(meta_writer, &(dimension, metric as u8, enable_vector_storage))?;
```

### Existing epoch-guard safety pattern for resize/snapshot tests
```rust
// Source: crates/velesdb-core/src/storage/guard.rs
let current = self.epoch_ptr.load(Ordering::Acquire);
assert!(current == self.epoch_at_creation, "Mmap was remapped; VectorSliceGuard is invalid");
```

## State of the Art

| Old Approach | Current Approach | When Changed | Impact |
|--------------|------------------|--------------|--------|
| Monolithic parser file | Incremental parser extraction (`match_parser.rs` already split out) | Already in current repo state | Confirms phased extraction is viable without API churn |
| Monolithic HNSW API implementation | Concern-based HNSW index module split (`index/constructors.rs`, `search.rs`, `batch.rs`) | Already in current repo state | Provides direct template for graph.rs extraction |
| Comment-only lock order guidance | Lock-order comments + concurrent deadlock tests | Existing `native/tests.rs` BUG-CORE-001 tests | Ready base for adding runtime checker and counters |

**Deprecated/outdated:**
- Relying on comments alone for concurrency safety; this phase should elevate lock ordering into runtime-checked invariants.

## Open Questions

1. **Delete semantics in "parallel insert/search/delete" family**
   - What we know: `NativeHnsw` graph has insert/search only; delete exists as soft-delete in `HnswIndex::remove`.
   - What's unclear: whether family-1 tests should target `NativeHnswIndex`, `HnswIndex`, or both.
   - Recommendation: plan tests at `HnswIndex` level for delete semantics plus `NativeHnsw` level for insert/search contention.

2. **Release checker overhead threshold**
   - What we know: lock-rank checks and atomic counters are cheap enough for release parity.
   - What's unclear: acceptable overhead budget for optional deep invariant checks.
   - Recommendation: keep rank checks and counters always-on; gate deep adjacency scans behind debug/test feature only.

## Sources

### Primary (HIGH confidence)
- `crates/velesdb-core/src/simd_native.rs` - function group boundaries and extraction targets (2406 lines)
- `crates/velesdb-core/src/index/hnsw/native/graph.rs` - lock ordering, graph operations, safety seams (640 lines)
- `crates/velesdb-core/src/velesql/parser/select.rs` - clause parsing and validation seams (828 lines)
- `crates/velesdb-core/src/velesql/parser/mod.rs` - parser modularization pattern already used
- `crates/velesdb-core/src/velesql/parser/conditions.rs` - shared parser helper pattern
- `crates/velesdb-core/src/velesql/parser/values.rs` - shared parser helper pattern
- `crates/velesdb-core/src/index/hnsw/native/tests.rs` - existing deadlock/concurrency tests baseline
- `crates/velesdb-core/src/index/hnsw/index/constructors.rs` - save/load duplication target
- `crates/velesdb-core/src/index/hnsw/native_index.rs` - save/load duplication target
- `crates/velesdb-core/src/storage/guard.rs` - epoch guard behavior and invariants
- `crates/velesdb-core/src/storage/tests.rs` - compaction+resize integrity baseline
- `crates/velesdb-core/src/storage/loom_tests.rs` - lock/epoch concurrency modeling pattern
- `.planning/ROADMAP.md` - phase goals and success criteria
- `.planning/REQUIREMENTS.md` - requirement IDs and acceptance framing

### Secondary (MEDIUM confidence)
- Rust Book: https://doc.rust-lang.org/book/title-page.html (ownership/borrowing constraints for lock helper design)

### Tertiary (LOW confidence)
- None.

## Metadata

**Confidence breakdown:**
- Standard stack: HIGH - directly from workspace/crate manifests and in-repo usage.
- Architecture: HIGH - extraction and lock-safety patterns are already demonstrated in adjacent modules.
- Pitfalls: HIGH - derived from concrete existing tests, known lock-order comments, and current duplication seams.

**Research date:** 2026-02-06
**Valid until:** 2026-03-08 (30 days)
