# Phase 4 — Verification Manifest

**Status**: Phase 4 (BM25 O(1) cold-start + sparse search speedup + HNSW
sequential-loop prefetch) did not follow the `perf_phase_gate.py`
before/after JSON capture pattern used for Phases 1–3. This manifest
documents the verification that *was* performed, so reviewers can audit
the evidence behind the CHANGELOG claims without having to trust headline
numbers.

Future phases must use `python scripts/perf_phase_gate.py capture
--phase <ID> --stage before` before any code change, per
[`.claude/rules/perf-phase-gate.md`](../../.claude/rules/perf-phase-gate.md).

---

## Phase 4.1 — BM25 persistence cold-start O(N) → O(1)

**PRs**: `#619` (impl), `#620` (docs / `KNOWN_LIMITATIONS.md` removal).

**Claim (CHANGELOG L230-235)**: BM25 index load no longer iterates every
persisted document; a single header read rebuilds the in-memory index.
Complexity goes from O(N) to O(1) with respect to document count.

**Evidence**:
- Implementation change in `crates/velesdb-core/src/index/bm25_persistence.rs`
  (header-only read path) — inspectable in `#619` diff.
- Correctness regression coverage: `crates/velesdb-core/src/index/bm25_persistence_tests.rs`
  and `bm25_persistence_wal.rs` exercise save/load round-trips and assert
  identical scoring before/after reload.
- `KNOWN_LIMITATIONS.md` entry removed by `#620` — explicit retirement of
  the prior O(N) caveat.

**Wall-clock reproducer**: not committed for v1.13.0. The complexity
improvement is proven by the code change itself (the loop is gone, not
tuned). A dedicated cold-start bench (persist 100K / 1M docs, measure
reload wall-clock) is planned for v1.13.x — see `bm25_cold_start.rs`
placeholder in the tracking issue.

---

## Phase 4.2 — Sparse search 16× speedup

**PR**: `#621` (closes `#378`).

**Claim (CHANGELOG L220-228)**: `sparse_search(top-10, 10K docs, SPLADE)`
drops from ≈956 µs → ≈57.6 µs; top-100 drops 927 µs → 75.1 µs. Driver:
small-corpus routing to a cache-resident linear scan (dense accumulator
stays L2-hot) in lieu of MaxScore DAAT overhead.

**Evidence**:
- Bench: `crates/velesdb-core/benches/sparse_benchmark.rs`
  `sparse_search::{top10,top100}_10k_corpus`.
- Routing rationale + empirical numbers documented in
  `crates/velesdb-core/src/sparse_index/search/mod.rs` (see
  `SMALL_CORPUS_LINEAR_THRESHOLD` doc-comment).
- **Recall validation** (added alongside this manifest): the bench now
  runs brute-force top-k ground truth over 20 sampled queries for both
  k=10 and k=100, and asserts `recall >= 0.95` before timed measurement.
  Measured recall on the synthetic SPLADE-like corpus: **1.0000 / 1.0000**.
  This proves the speedup preserves scoring semantics — it is not a
  "made search wrong and called it fast" artefact.

**Corpus caveat**: the bench corpus is synthetic SPLADE-like (random
term IDs + weights). It exercises the same code path as real SPLADE but
production retrieval quality on realistic data is validated separately
(`cargo test test_recall` and `cargo bench sift1m_recall`).

---

## Phase 4.3 — HNSW sequential-loop prefetch

**PR**: `#623` (progresses `#611`).

**Claim (CHANGELOG L210-215)**: −12 % to −22 % search latency on HNSW
for datasets > 10K vectors via peek-based speculative prefetch during
graph traversal (arXiv:2505.07621).

**Evidence**:
- Bench: `crates/velesdb-core/benches/hnsw_benchmark.rs` and
  `prefetch_tuning_benchmark.rs` (prefetch stride tuning).
- Recall preserved via the standing CI gate
  `cargo test -p velesdb-core test_recall` (≥ 0.95 @ 10K,
  see `.claude/rules/recall-quality-gate.md`).

---

## Next-time checklist (mandatory for Phase 5+)

1. `python scripts/perf_phase_gate.py capture --phase N --stage before`
   BEFORE the first commit of the phase.
2. Implement + review + merge.
3. `python scripts/perf_phase_gate.py gate --phase N` — exit code 0 or
   the phase does not ship.
4. Commit both JSONs under `benchmarks/phase_results/phase_N_{before,after}.json`.
5. Cite the JSON paths in the CHANGELOG entry, not just the headline number.
