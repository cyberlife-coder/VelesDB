# Phase 3: Architecture Extraction & Graph Safety - Context

**Gathered:** 2026-02-06
**Status:** Ready for planning

<domain>
## Phase Boundary

Improve maintainability by extracting oversized modules into coherent sub-modules and strengthen HNSW concurrent access safety.

Scope is fixed to Phase 3 roadmap outcomes: module extraction, deduplication in-scope refactors, HNSW lock-ordering safety, and concurrent resize test coverage. No new product capabilities are added in this phase.

</domain>

<decisions>
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

</decisions>

<specifics>
## Specific Ideas

- "Hybrid" parser decomposition expectation: by clause plus shared validation logic.
- HNSW safety should be auditable in practice: invariants + concurrency scenarios + observable counters.
- Release builds should preserve strong safety checks, not debug-only confidence.

</specifics>

<deferred>
## Deferred Ideas

None - discussion stayed within Phase 3 scope.

</deferred>

---

*Phase: 03-architecture-extraction-graph-safety*
*Context gathered: 2026-02-06*
