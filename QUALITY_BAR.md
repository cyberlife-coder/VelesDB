# VelesDB Quality Bar

This document specifies the **explicit, enforceable thresholds** below which VelesDB does not ship. Every release passes every gate, or the release is blocked.

These gates are not aspirational. They are enforced via CI workflows, scripts, and explicit pre-merge protocols. Each gate listed here links to its enforcement mechanism so that the gate can be inspected, contested, or extended publicly.

> **Last updated:** 2026-04-27 — applies to v1.13.x and onward.

---

## TL;DR — the seven non-negotiable gates

| # | Gate | Threshold | Enforced by |
|---|------|-----------|-------------|
| 1 | **Recall@10** | ≥ 0.95 (10K local) ; ≥ 0.90 (100K CI) | `cargo test test_recall` + GitHub Actions `recall-quality` job |
| 2 | **End-to-end search latency p50** | ≤ 450 µs (10K/384D, WAL ON, recall ≥ 96%) | `python benchmarks/velesdb_benchmark.py --recall` + perf-smoke CI gate |
| 3 | **No `.unwrap()` in production code** | Zero | `scripts/check_prod_unwraps.py` in CI |
| 4 | **No `unsafe` without `// SAFETY:` comment** | Zero | `scripts/verify_unsafe_safety_template.py` in CI |
| 5 | **Cyclomatic complexity** | ≤ 8 per function | Codacy Cloud (blocking) |
| 6 | **Function NLOC** | ≤ 50 | Codacy Cloud (blocking) |
| 7 | **Code duplication** | < 2% per language | jscpd in `scripts/local-ci.ps1` + Codacy |

A pull request that breaks any of these gates **cannot be merged**. There are no waivers without a documented exception added to this file.

---

## Gate 1 — Recall@10 ≥ 0.95

**Why:** A 5% recall drop on 10K vectors compounds catastrophically at production scale (1M+ vectors with the same drop becomes effectively unusable). Performance optimizations that silently degrade recall are the most dangerous regressions in a vector database.

**Enforcement:**

- **Local (mandatory before every PR touching the search path):**
  ```bash
  cargo test -p velesdb-core --features persistence test_recall -- --test-threads=1
  ```
  Must pass with recall ≥ 0.95 on 10K vectors.

- **CI (authoritative):** `recall-quality` job runs on every push to `develop` and `main` with 100K vectors and threshold ≥ 0.90.

- **Documented modes:**

  | Mode | ef_search | Recall@10 (measured) |
  |------|-----------|---------------------|
  | Fast | 64 | 92.2% |
  | Balanced (default) | 128 | 98.8% |
  | Accurate | 512 | 100.0% |

  Source: `benchmarks/results/2026-02-20-phase-e-report.md`.

**When this triggers:** any change to `index/hnsw/`, `simd_native/`, `quantization/`, `fusion/`, or result-conversion code in Python bindings.

**See also:** [`.claude/rules/recall-quality-gate.md`](.claude/rules/recall-quality-gate.md) (internal rule), [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md).

---

## Gate 2 — End-to-end search latency p50 ≤ 450 µs

**Why:** This is the **canonical claim** in the README and in marketing material (Quick Comparison table). It is the full production path: VelesQL parse + plan + WAL fsync + HNSW search + recall ≥ 96%. We do not ship a release that breaks this number.

**Threshold:** 450 µs p50 on 10K vectors / 384D with WAL ON, measured on the i9-14900KF reference machine.

**Enforcement:**

- **Reproducible benchmark:** `python benchmarks/velesdb_benchmark.py --recall`
- **Source:** `CHANGELOG.md` v1.13.0 (measured 2026-03-27, baseline preserved through pre-seed remediation phases)
- **Promise contract:** [`docs/reference/promise-contract.json`](docs/reference/promise-contract.json) entry `readme_production_search_latency` enforces the exact substring `**450 us**` in `README.md`.

**Index-only micro-benchmarks** (separately measured, separately labeled in README):

| Component | Threshold | Reproduce |
|-----------|-----------|-----------|
| HNSW Search index-only (5K/768D, k=10) | ≤ 60 µs | `cargo bench -p velesdb-core --bench hnsw_benchmark` |
| SIMD Dot Product (768D, AVX2) | ≤ 25 ns | `cargo bench -p velesdb-core --bench simd_benchmark` |
| BM25 Sparse Search index-only (10K, top-10) | ≤ 70 µs | `cargo bench -p velesdb-core --bench sparse_benchmark` |

These are **not the same number** as the canonical 450 µs. The README explicitly disambiguates them since v1.13.3.

**See also:** [`.claude/rules/perf-phase-gate.md`](.claude/rules/perf-phase-gate.md) (internal rule).

---

## Gate 3 — No `.unwrap()` in production code

**Why:** `.unwrap()` panics at runtime. A panic in a vector database means a process crash, which means data loss risk on writes and downtime on reads. We treat any production unwrap as a hard CI failure.

**Threshold:** Zero unwraps in any file outside `tests/`, `_tests.rs`, `benches/`, `examples/`, or any `#[cfg(test)]` block.

**Enforcement:**

- **CI script:** `scripts/check_prod_unwraps.py` — runs on every push, blocks merge if non-zero.
- **Status:** **PASSED** as of v1.13.2.

**Approved alternatives** (in order of preference):

| Instead of | Use | When |
|------------|-----|------|
| `.unwrap()` | `?` operator | Function returns `Result` or `Option` |
| `.unwrap()` | `.expect("invariant: <reason>")` | Value is guaranteed by logic, document why |
| `.unwrap()` | `.unwrap_or(default)` | Sensible default exists |
| `.unwrap()` | `.unwrap_or_else(\|\| ...)` | Default needs computation |

**Lock acquisition:** we use `parking_lot::RwLock` / `Mutex` exclusively. These never poison, so `.read()` / `.write()` / `.lock()` return guards directly with no `.unwrap()`.

**See also:** [`.claude/rules/rust-safety.md`](.claude/rules/rust-safety.md) (internal rule).

---

## Gate 4 — Every `unsafe` block has a `// SAFETY:` comment

**Why:** `unsafe` is allowed only where it is necessary for performance (SIMD intrinsics, mmap, FFI). Every such block must be auditable: a maintainer reading the code in two years must understand why this is safe.

**Threshold:** Zero unsafe blocks without a corresponding `// SAFETY:` comment.

**Enforcement:**

- **CI script:** `scripts/verify_unsafe_safety_template.py` — runs on every push, blocks merge if any `unsafe {` is not preceded or annotated with `// SAFETY:`.
- **Coverage:** 129 unsafe blocks workspace-wide, 431 SAFETY comments (ratio > 3:1, well-documented).
- **Audit trail:** [`docs/SOUNDNESS.md`](docs/SOUNDNESS.md) documents invariants for every unsafe pattern in the codebase (964 lines).

**External audit:** Planned in v1.15 horizon (Cure53 / independent Rust safety expert), conditional on funding.

---

## Gate 5 — Cyclomatic complexity ≤ 8

**Why:** Functions with CC > 8 are the leading source of bugs in any codebase (Fowler, *Refactoring*). We enforce the limit aggressively.

**Threshold:** Cyclomatic complexity ≤ 8 per function, all languages.

**Enforcement:**

- **Codacy Cloud (authoritative, blocking):** any PR introducing a function with CC > 8 fails the Codacy gate, blocks merge.
- **Local CLI (lizard, threshold > 15):** advisory, used during development.
- **Refactoring pattern:** Extract Function (Fowler #106) is the primary tool — if a fragment can be named, extract it.

**See also:** [`.claude/rules/code-quality.md`](.claude/rules/code-quality.md), [`.claude/rules/rust-clean-code.md`](.claude/rules/rust-clean-code.md).

---

## Gate 6 — Function NLOC ≤ 50

**Why:** Long functions hide complexity. A 50-NLOC ceiling forces helper extraction and improves testability.

**Threshold:** Function Non-comment Lines of Code ≤ 50.

**Enforcement:**

- **Codacy Cloud (authoritative, blocking):** PR fails if any new function exceeds 50 NLOC.
- **Exception:** Codacy `.codacy.yml` excludes test files (`_tests.rs`, `tests/`) from this rule because Arrange-Act-Assert blocks legitimately span more lines.

**Known violations (file NLOC > 500, file-level limit):**

| File | NLOC | Plan |
|------|------|------|
| `simd_native/x86_avx512.rs` | 1468 | Hard to split (intrinsics block); accepted exception |
| `simd_native/neon.rs` | 902 | Same as above |
| `velesdb-server/src/config.rs` | 837 | v1.15 — refactor planned |
| `velesdb-migrate/src/pipeline.rs` | 806 | v1.15 — refactor planned |

These are tracked but do not block release because they are SIMD intrinsics blocks (function-level NLOC is fine; file-level breach is unavoidable for hand-written intrinsics).

---

## Gate 7 — Code duplication < 2%

**Why:** DRY violations multiply maintenance cost. Each duplicated 50-token block becomes 2x the work for every future change.

**Threshold:** < 2% duplicated lines per language (Rust, Python, TypeScript).

**Enforcement:**

- **Local:** `npx jscpd --min-tokens 50 --reporters console --format rust crates/` — integrated in `scripts/local-ci.ps1`.
- **Codacy Cloud:** server-side check, blocking.

**Status:** Within budget across all crates as of v1.13.2.

**See also:** [`.claude/rules/no-duplication.md`](.claude/rules/no-duplication.md).

---

## Additional gates (advisory, not blocking)

These signals are tracked but do not block release individually:

| Signal | Threshold | Enforcement |
|--------|-----------|-------------|
| Test count | ≥ 7000 (Rust + TS + Py) | Counted in CI summary |
| BDD scenario coverage | All VelesQL syntax features | `crates/velesdb-core/tests/bdd/` |
| Doctest compilation | All `pub fn` doctests compile | `cargo test --doc` in CI |
| Promise contract sync | All numeric claims in README backed by benchmark commands | `scripts/check-promise-contract.py` in CI |
| TODO governance | All TODOs in format `// TODO(EPIC-XXX):` | `scripts/check-todo-annotations.py` |
| RUSTSEC | All advisories tracked or justified in `deny.toml` | `cargo deny check` in CI (Security Audit job) |

---

## Pre-release final checklist

Before tagging any release (patch, minor, major), all of the following must be **green**:

- [ ] Local: `cargo fmt --all -- --check`
- [ ] Local: `cargo clippy --workspace --all-targets --features persistence,gpu,update-check --exclude velesdb-python -- -D warnings -D clippy::pedantic`
- [ ] Local: `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1`
- [ ] Local: `python scripts/check_prod_unwraps.py`
- [ ] Local: `python scripts/check-promise-contract.py`
- [ ] Local: `wsl -- bash -c "codacy-cli analyze"` (Codacy CLI in WSL)
- [ ] CI on `develop`: all jobs green for last 3 commits
- [ ] If search path touched: recall test ≥ 0.95
- [ ] If perf optimization: `python scripts/perf_phase_gate.py gate --phase <ID>` exit code 0
- [ ] CHANGELOG.md updated with conventional commit subject groups
- [ ] All numeric claims in CHANGELOG/README updated in `promise-contract.json`
- [ ] Devin Review on the release PR: clean
- [ ] Codacy Cloud on the release PR: 0 blocking findings

The full checklist is automated in `.github/workflows/release.yml` after merge to `main` triggers tag publishing.

---

## How to propose changing this bar

The quality bar is intentionally hard to lower. To change a threshold:

1. Open a GitHub Discussion explaining why the current threshold is wrong (data, not opinion).
2. If the change relaxes a threshold, the discussion must include a **migration plan** to compensate (e.g. "we lower CC ≤ 8 to ≤ 10 in the SIMD module specifically, with an explicit `.codacy.yml` exception, because intrinsics block needs more branches").
3. If the change tightens a threshold, the discussion includes a **timeline for compliance** across the existing codebase.
4. The founder makes the final call. The decision is recorded in `CHANGELOG.md` under the relevant release.

---

## Related documents

- [`ROADMAP.md`](ROADMAP.md) — what we are building, when
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to contribute under these gates
- [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md) — current technical limitations
- [`.claude/rules/`](.claude/rules/) — internal rules backing these gates (kept local; this public file is the externally-visible summary)
