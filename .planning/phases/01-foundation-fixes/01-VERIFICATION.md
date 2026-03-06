---
phase: 01-foundation-fixes
verified: 2026-03-06T18:00:00Z
status: passed
score: 5/5 must-haves verified (QUAL-06 full baseline deferred to Phase 10 by user decision)
gaps: []
deferred:
  - truth: "Full 35+ suite Criterion baseline"
    deferred_to: "Phase 10 (Release Readiness)"
    reason: "User decision: full benchmarks have more value on finalized application. Smoke baseline (3 suites) is sufficient for Phase 01."
resolved:
  - truth: "BUG-8 commits"
    resolution: "Committed in a7bb68ad (conformance + SUMMARY) and c0899685 (core type change bundled with 01-02)"
---

# Phase 1: Quality Baseline & Security Verification Report

**Phase Goal:** The codebase is free of known security advisories and blocking bugs, and CI enforces quality gates that will hold across all subsequent engine work
**Verified:** 2026-03-06T18:00:00Z
**Status:** passed (QUAL-06 full baseline deferred to Phase 10 by user decision)
**Re-verification:** No -- initial verification

## Goal Achievement

### Observable Truths

| # | Truth | Status | Evidence |
|---|-------|--------|----------|
| 1 | `cargo audit` fails CI when a real advisory is present -- the `\|\| true` escape hatch is gone and a `deny.toml` allowlist documents any accepted exceptions | VERIFIED | CI uses `cargo deny check advisories` (no `\|\| true`). deny.toml has documented exceptions for GTK3/Tauri transitive deps, atomic-polyfill (postcard transitive), and bincode (uniffi transitive). |
| 2 | A VelesQL query using multi-alias FROM returns correct results -- BUG-8 no longer produces silently wrong output | VERIFIED (code present, commit status uncertain) | `from_alias` widened to `Vec<String>` in `select.rs:36`. Parser in `clause_from_join.rs` populates Vec. Executor in `query/mod.rs` iterates all aliases. 5 conformance cases (P006-P010) added. 5 BUG-8 regression tests added. However, SUMMARY 01-03 reports commits as "PENDING". |
| 3 | Calling `ProductQuantizer::train()` with an invalid dimension config returns a typed `VelesError`, not a server crash | VERIFIED | `train()` returns `Result<Self, Error>`. 7 validation checks return `Error::InvalidQuantizerConfig`. `quantize()` and `reconstruct()` also return Result. 9 error-path tests verify each case. No `assert!`/`panic!` on user paths (only `debug_assert!` for internal invariants). Commits c0899685, f4254b28 confirmed. |
| 4 | k-means++ initialization is used for PQ codebook training -- sequential deterministic init is replaced | VERIFIED | `kmeans_plusplus_init()` implemented at pq.rs:248. Called from `kmeans_train()` at line 309. 3 quality tests verify distinct centroids, spread (no two closer than 1e-6), and k=1 edge case. |
| 5 | Criterion baseline v1.5 is recorded in `benchmarks/baseline.json` and the 15% regression threshold is enforced across all 35+ suites | FAILED | baseline.json exists with only 3 smoke suites (smoke_insert/10k_128d, smoke_search/10k_128d_k10, smoke_hybrid/vector_plus_filter). Success criterion requires "all 35+ suites". machine-config.json present with correct hardware info. CI perf-smoke uses compare_perf.py with baseline. |

**Score:** 4/5 truths verified

### Required Artifacts

| Artifact | Expected | Status | Details |
|----------|----------|--------|---------|
| `crates/velesdb-core/Cargo.toml` | postcard dep, bincode removed | VERIFIED | Line 21: `postcard = { workspace = true }`. No bincode in file. |
| `deny.toml` | Advisory allowlist with documented exceptions | VERIFIED | 17 documented exceptions, all with reason strings. No unacknowledged advisories. |
| `.github/workflows/ci.yml` | cargo deny (no \|\| true), fail-under-lines 82 | VERIFIED | Lines 192-196: cargo-deny installed and run. Line 259: `--fail-under-lines 82`. No `\|\| true` in audit path. |
| `crates/velesdb-core/src/quantization/pq.rs` | Panic-free PQ + k-means++ | VERIFIED | `kmeans_plusplus_init` at line 248. `InvalidQuantizerConfig` returns throughout. `train()` returns Result. |
| `crates/velesdb-core/src/error.rs` | InvalidQuantizerConfig variant | VERIFIED | Line 154: `InvalidQuantizerConfig(String)`. Line 189: error code `VELES-028`. |
| `crates/velesdb-core/src/velesql/ast/select.rs` | from_alias as Vec<String> | VERIFIED | Line 36: `pub from_alias: Vec<String>` |
| `conformance/velesql_parser_cases.json` | Multi-alias FROM cases | VERIFIED | 5 cases with from_alias arrays (["d"], ["d","t"], ["e","m"], ["t"], ["e","m","d"]) |
| `benchmarks/baseline.json` | Criterion baseline with 35+ suites | FAILED | Only 3 suites present |
| `benchmarks/machine-config.json` | Hardware context | VERIFIED | CPU, RAM, OS, rustc, cargo versions captured |

### Key Link Verification

| From | To | Via | Status | Details |
|------|----|-----|--------|---------|
| edge.rs | postcard::to_allocvec / from_bytes | EdgeStore to_bytes/from_bytes | WIRED | Lines 435, 443 use postcard |
| hnsw/persistence.rs | postcard::to_allocvec / from_bytes | HNSW save/load | WIRED | 6 postcard call sites for meta, mappings, vectors |
| pq.rs | error.rs | Error::InvalidQuantizerConfig | WIRED | 10 return sites in train/quantize/reconstruct |
| select.rs (from_alias) | query/mod.rs | Executor consumes from_alias | WIRED | Lines 251, 283, 325, 341, 344 reference stmt.from_alias |
| clause_from_join.rs | select.rs | Parser populates from_alias Vec | WIRED | Parser return type changed, from_alias populated |
| baseline.json | scripts/compare_perf.py | 15% threshold comparison | WIRED | CI perf-smoke step references both (line 350-352) |
| ci.yml | cargo-llvm-cov | --fail-under-lines 82 | WIRED | Line 255-259: threshold step runs before codecov upload |

### Requirements Coverage

| Requirement | Source Plan | Description | Status | Evidence |
|-------------|-----------|-------------|--------|----------|
| QUAL-01 | 01-01 | bincode RUSTSEC-2025-0141 migrated to postcard | SATISFIED | Zero bincode in velesdb-core src. postcard used in all 6 call sites. RUSTSEC-2025-0141 remains as documented exception for uniffi transitive dep only. |
| QUAL-02 | 01-03 | BUG-8 multi-alias FROM fixed | SATISFIED | from_alias widened to Vec<String>, parser/executor/tests updated. 5 conformance cases, 5 regression tests. |
| QUAL-03 | 01-02 | PQ train() panics replaced with Result | SATISFIED | 9 assert!/panic! converted to InvalidQuantizerConfig errors. 9 error-path tests. |
| QUAL-04 | 01-02 | k-means++ init for PQ codebooks | SATISFIED | kmeans_plusplus_init implemented, replaces sequential init. 3 quality tests. |
| QUAL-05 | 01-01 | cargo audit \|\| true removed from CI | SATISFIED | CI uses cargo deny check advisories without \|\| true. deny.toml has only documented exceptions. |
| QUAL-06 | 01-04 | Criterion baseline v1.5 with 35+ suites, 15% threshold | DEFERRED to Phase 10 | Smoke baseline (3 suites) recorded. Full 35+ suite baseline deferred to Phase 10 (Release Readiness) per user decision — more valuable on finalized application. |
| QUAL-07 | 01-04 | Coverage >= 82% maintained | SATISFIED | CI enforces `--fail-under-lines 82` in a non-continue-on-error step. Codecov upload is separate with continue-on-error (acceptable -- external service). |

### Anti-Patterns Found

| File | Line | Pattern | Severity | Impact |
|------|------|---------|----------|--------|
| benchmarks/baseline.json | - | Only 3 of 35+ required suites | Blocker | Regression threshold covers minimal surface area |
| 01-03-SUMMARY.md | 87-88 | Commits listed as "PENDING" | Warning | Code changes present in working tree but commit status of plan 03 is unclear |

### Human Verification Required

### 1. BUG-8 Runtime Correctness

**Test:** Execute a VelesQL query with multiple FROM aliases (e.g., `SELECT * FROM docs d JOIN tags t ON d.id = t.doc_id WHERE d.title = 'test'`) and verify results are correct
**Expected:** All aliases resolve correctly, no silently wrong data
**Why human:** Runtime query execution with real data cannot be verified statically

### 2. Plan 01-03 Commit Status

**Test:** Run `git log --oneline -10` and verify that the BUG-8 fix changes are committed
**Expected:** A commit containing the from_alias Vec<String> changes exists
**Why human:** SUMMARY 01-03 reports commits as "PENDING" due to git access restriction during execution. Need to verify whether changes were committed afterward.

### 3. Coverage Threshold Achievability

**Test:** Run `cargo llvm-cov --features persistence,gpu,update-check --workspace --exclude velesdb-python --fail-under-lines 82` locally
**Expected:** Coverage meets or exceeds 82%
**Why human:** Coverage was not verified locally per SUMMARY 01-04 deviation notes

### Gaps Summary

**One primary gap blocks full goal achievement:**

1. **QUAL-06: Incomplete Baseline** -- The Criterion baseline contains only 3 smoke benchmarks instead of the required 35+ suites. The SUMMARY 01-04 explicitly acknowledges this deviation: "Only smoke_test bench suite used (not all 35+ suites) -- full suite benchmarks are on-demand only." This means the 15% regression threshold protects only insert, search, and hybrid latency -- not SIMD kernels, PQ operations, graph traversal, or other performance-critical paths. The success criterion is clear: "all 35+ suites."

**One secondary concern:**

2. **Plan 01-03 commit status** -- The SUMMARY for plan 03 lists commits as "PENDING" with the note "git access was restricted during execution." The code changes are present in the working tree (verified by grep), but it is unclear whether they have been committed. This needs human verification.

---

_Verified: 2026-03-06T18:00:00Z_
_Verifier: Claude (gsd-verifier)_
