# VelesDB Roadmap

This roadmap commits to **what we are building**, **why**, and **when**. It is updated each minor release and synced with the GitHub Milestones.

It is intentionally narrow. Items not on this roadmap are tracked as `roadmap` issues but **not committed** until they reach a milestone here.

> **Last updated:** 2026-06-21 — covers v3.5.0 (current). The horizon framing below predates the v2.0→v3.x line and is being re-baselined.

---

## Horizon 1 — ✅ Shipped (v1.14.x → v1.15.0, released 2026-05-14)

### Theme: Ecosystem credibility & adoption signals

| # | Item | Success criterion | Status |
|---|------|------------------|--------|
| 1 | [Haystack 2.x DocumentStore integration (#349)](https://github.com/cyberlife-coder/VelesDB/issues/349) | At least one BDD test passing in CI; published in `integrations/haystack/`; first community contribution merged | ✅ Shipped in v1.14.0 / v1.14.1 — PR [#672](https://github.com/cyberlife-coder/VelesDB/pull/672) by [@CrepuscularIRIS](https://github.com/CrepuscularIRIS); `pip install haystack-velesdb` live on PyPI |
| 2 | [Onboarding time-to-first-search < 5 min (#379)](https://github.com/cyberlife-coder/VelesDB/issues/379) | Measured on clean Ubuntu/macOS/Windows; documented in `docs/quickstart/timing-results.md`; verified via reproducible Docker harness | ✅ Shipped in v1.13.7 (Phase 6) — median across 4 paths under 26 s; [`scripts/dx-timing/run_all.sh`](scripts/dx-timing/run_all.sh) reproduces it |
| 3 | [CBO calibration loop (#469)](https://github.com/cyberlife-coder/VelesDB/issues/469) | `COST_UNIT_TO_MS` recalibrated from real query timings; method documented in `BENCHMARKS.md`; removes `KNOWN_LIMITATIONS.md #1` | ⚠️ Phase 2 shipped in v1.15.0 — empirical EMA reported in `EXPLAIN ANALYZE` output (PR [#784](https://github.com/cyberlife-coder/VelesDB/pull/784)); full `COST_UNIT_TO_MS` empirical pin and removal of `KNOWN_LIMITATIONS.md #1` carried forward (Horizon 4) |
| 4 | [Python DataFrame + Polars integration (#429)](https://github.com/cyberlife-coder/VelesDB/issues/429) | `upsert_dataframe(df)` + `search().to_polars()` round-trip; one notebook in `examples/python/` | ✅ Shipped — `Collection.upsert_from_dataframe(df)` (pandas/polars auto-detected) and `to_dataframe(backend="polars")` round-trip via `velesdb.dataframe_converter`. |
| 5 | [PyO3 SearchOptions builder (#717)](https://github.com/cyberlife-coder/VelesDB/issues/717) | Replace the wide-kwarg `Collection.search` signature with a builder pattern + deprecation cycle; remove the `clippy::too_many_arguments` allow-list | ✅ Shipped in v1.15.0 — fluent `SearchOptions` builder exposed in Python SDK (PR [#761](https://github.com/cyberlife-coder/VelesDB/pull/761), closes #717) |

**Also shipped in v1.14.x:** MSRV bump 1.83 → 1.89 (#714), Dockerfile auto-sync (#715), full Python RAG framework trio (Haystack via #672), doc consistency sweep (#722), `haystack-velesdb` PyPI publishing (#723), Haystack `DuplicatePolicy.SKIP` contract fix (#726), full v1.14.2 doc alignment + fictional MSI installer removed + 14-entry tooling extension (#730), Haystack runtime gaps closed — `@component` decorator on retriever example + Haystack-filter→VelesDB-filter translator + real-Haystack CI (#731).

**Also shipped in v1.15.0:** ACT-R Phase 1 procedural learning (PR [#780](https://github.com/cyberlife-coder/VelesDB/pull/780)); Python auto-detect vector dimension from first upsert (PR [#778](https://github.com/cyberlife-coder/VelesDB/pull/778)); `IN` filter O(log n) binary search (PR [#765](https://github.com/cyberlife-coder/VelesDB/pull/765)); React+WASM and Node+Express RAG demos; `rand 0.9 → 0.10` and `toml 0.8 → 0.9` ecosystem majors absorbed.

---

## Horizon 2 — ✅ Shipped (v1.15.0 → v1.16.0, released 2026-05-30)

### Theme: Performance narrative & SDK parity

| # | Item | Why | Status |
|---|------|-----|--------|
| 1 | [HNSW <30µs index-only target (#377)](https://github.com/cyberlife-coder/VelesDB/issues/377) | Push the index-only micro-bench from 55µs to <30µs to widen the headroom on the 450µs full-path number | ✅ Shipped in v1.15.0 — `ANALYZE` now triggers in-place HNSW node reorder when fragmentation exceeds threshold; 10K-probe recall@10 off-by-one fixed (PR [#785](https://github.com/cyberlife-coder/VelesDB/pull/785), closes #377) |
| 2 | [SDK parity: TypeScript/LangChain/LlamaIndex (#380)](https://github.com/cyberlife-coder/VelesDB/issues/380) | Close the cross-language gap so any framework user gets the same API surface | ✅ Shipped in v1.15.0 — TypeScript REST backend gains `sparseIndexName` and RSF weights (PR [#779](https://github.com/cyberlife-coder/VelesDB/pull/779), closes #380) |
| 3 | **Reproducible head-to-head benchmark vs Qdrant + Chroma + pgvector** (Docker Compose) | Pre-seed audit P0: turn marketing claims into proven numbers | ⚠️ Partially shipped — a reproducible Docker Compose head-to-head vs pgvector lives in `benchmarks/` (docker-compose.yml + benchmark_docker.py); the Qdrant + Chroma legs are still pending. |
| 4 | **External `unsafe` audit** (SIMD module, Cure53 / independent Rust safety expert) | Required for "data sovereignty" enterprise positioning | ❌ Pending funding (~5-15 k€); carried forward (Horizon 4) |
| 5 | **`velesdb-migrate` rework decision** (12,108 LOC, 9 connectors) | Workspace inflation without measured user base — decide keep / extract / archive based on crates.io download counts, GitHub stars attributable to migration tooling, opened issues count. See `docs/reference/KNOWN_LIMITATIONS.md` § 4 | ❌ Decision deferred — carried forward (Horizon 4) |

**Also shipped in v1.16.0 (2026-05-30):** `audit-2026q2` security hardening wave (9 PRs: HNSW on-disk validation, WAL allocation caps, PQ hardening, parser DoS bounds, sparse/BM25 agent path, graph integrity, query/cache, config validation, rate limiter — PRs [#908](https://github.com/cyberlife-coder/VelesDB/pull/908)–[#916](https://github.com/cyberlife-coder/VelesDB/pull/916)); first-party Python + TypeScript embedding adapters (PR [#917](https://github.com/cyberlife-coder/VelesDB/pull/917)); multi-arch GHCR image with OIDC attestation; 9 typed Tauri guest-JS wrappers (PR [#928](https://github.com/cyberlife-coder/VelesDB/pull/928)); 44-PR dependency refresh (Docker base `rust 1.87→1.96`, `wgpu 29`, `redis 0.26→1.2`, `dashmap 5→6`, `uniffi 0.28→0.31`); VelesQL cheat sheet.

---

## Horizon 3 — ✅ Shipped (v1.16.0 → v1.18.0, released 2026-06-05 and 2026-06-07)

### Theme: Correctness, licensing & agent-memory parity

v1.17.0 and v1.18.0 shipped well ahead of the planned cadence and were dominated by correctness and parity work — none of the enterprise/DataFrame items originally penciled in for this horizon landed; all five carry forward to Horizon 4 below.

**Shipped in v1.17.0 (2026-06-05):** VelesQL parser error hints with did-you-mean suggestions ([#987](https://github.com/cyberlife-coder/VelesDB/pull/987)); payload-WAL torn-tail crash recovery ([#1011](https://github.com/cyberlife-coder/VelesDB/pull/1011)); hybrid fusion weight validation ([#1013](https://github.com/cyberlife-coder/VelesDB/pull/1013)) and HNSW `alpha` boundary validation ([#1015](https://github.com/cyberlife-coder/VelesDB/pull/1015)); OpenAPI id-type accuracy plus a CI drift check on the committed spec; HNSW search probe-RNG contention removed ([#1001](https://github.com/cyberlife-coder/VelesDB/pull/1001)).

**Shipped in v1.18.0 (2026-06-07):** engine-embedding artifacts realigned to VelesDB Core License 1.0 ([#1053](https://github.com/cyberlife-coder/VelesDB/pull/1053)); Python agent-memory bindings — TTL, snapshots, VelesQL bridges ([#1045](https://github.com/cyberlife-coder/VelesDB/pull/1045)); Tauri agent-memory commands ([#1046](https://github.com/cyberlife-coder/VelesDB/pull/1046)); agent-memory TTL & expiry hardening ([#1040](https://github.com/cyberlife-coder/VelesDB/pull/1040)/[#1043](https://github.com/cyberlife-coder/VelesDB/pull/1043)); TypeScript procedural recall fixed via required `embedding` ([#1039](https://github.com/cyberlife-coder/VelesDB/pull/1039)).

---

## Horizon 4 — Next (v1.18.0 → v1.19)

### Theme: Enterprise readiness & DataFrame ergonomics

Carry-forward open items from Horizons 1–3 plus the first enterprise-readiness primitives.

**Already queued for v1.19** (see `CHANGELOG.md` [Unreleased]): agent-memory graph dimension (`relate()` API — the flagship NEAR + MATCH agent-memory query runs verbatim), GraphFirst anchored hybrid retrieval, PQ/RaBitQ quantization wired end-to-end across restarts, durable TTL enforced on every read path, REST/TypeScript relations + TTL parity, `GET /metrics` served by default.

**Committed scope:**

| # | Item | Why |
|---|------|-----|
| 1 | [CBO calibration — full `COST_UNIT_TO_MS` pin (#469)](https://github.com/cyberlife-coder/VelesDB/issues/469) | Pin the cost constant from a micro-benchmark; removes `KNOWN_LIMITATIONS.md #1` |
| 2 | **Reproducible head-to-head benchmark vs Qdrant + Chroma + pgvector** (Docker Compose) | Pre-seed audit P0: turn marketing claims into proven numbers |
| 3 | **External `unsafe` audit** (SIMD module, Cure53 / independent Rust safety expert) | Required for "data sovereignty" enterprise positioning; budget-conditional (~5-15 k€) |
| 4 | **`velesdb-migrate` rework decision** | Evaluate keep / extract / archive based on crates.io download counts, GitHub stars, open-issue count |

**Tentative scope (enterprise feature gate):**

- **Concurrent WAL writer** with batching (today: single-writer mutex)
- **Multi-tenancy / namespacing** (today: per-database isolation only)
- **RBAC** (Role-Based Access Control) — premium companion crate
- **Distributed replication** (Raft) — premium, long horizon
- **Query result caching with auth tags** — premium

The [Deferred — Hardware accelerator backlog](#deferred--hardware-accelerator-backlog) will be re-evaluated at this horizon based on customer requests.

---

## Deferred — Hardware accelerator backlog

SIMD and GPU items that are part of the long-term roadmap but **explicitly on hold** until VelesDB has clearer signal on hardware-target priorities (cloud GPUs vs ARM vs legacy x86). Consolidated from individual issues during the 2026-04-27 pre-seed audit to keep the active roadmap visible vs the long-tail wishlist.

| Item | Rationale to defer |
|------|-------------------|
| `perf(gpu)`: CUDA/cuBLAS backend for NVIDIA GPUs | Existing wgpu pipeline (PR #626) covers cross-vendor GPU; CUDA-specific only valuable for NVIDIA-tied datacenter customers, defer until first such request |
| `perf(simd)`: SSE4.2 fallback kernels for legacy x86_64 | AVX2 covers ~99% of CPUs from 2013+, SSE4.2-only fallback ROI is minimal |
| `perf(simd)`: FP16 native SIMD kernels (AVX-512FP16, NEON fp16) | Niche compute; SQ8 quantization already covers most memory-bound use cases |
| `perf(gpu)`: batch Hamming & Jaccard distance compute shaders (WGSL) | Existing GPU SONG pipeline covers L2/cosine; binary metrics defer until user demand |
| `perf(simd)`: AVX-512-VNNI kernels for INT8 quantized distance | Niche; SQ8 + AVX2 already gives 4× compression at acceptable speed |
| `perf(gpu)`: async CPU/GPU pipelining with double buffering | Optimization on existing GPU path; defer until profiling shows it as a top bottleneck |
| `perf(simd)`: SVE kernels for AWS Graviton3/4 | NEON covers ARM today; SVE valuable only for AWS-Graviton-specific workloads |

**Re-activation criteria** — any of:
- A paying customer / design partner requests a specific item
- Profiling on a real workload identifies one as a top-3 bottleneck
- A community contributor opens a draft PR for one

**What is NOT deferred** (already shipped in v1.13.x): AVX-512 / AVX2 / NEON kernels for f32/f16 cosine·dot·euclidean, GPU SONG 3-stage pipeline (PR #626), GPU `search_auto` wiring (PR #638). WASM SIMD128 kernels remain planned (wasm32 currently uses the scalar fallback). Current SIMD coverage matrix in [`docs/reference/NATIVE_HNSW.md`](docs/reference/NATIVE_HNSW.md).

---

## What we are explicitly NOT doing

To make the roadmap meaningful, here is what is **out of scope** for the foreseeable future:

| Out of scope | Why |
|--------------|-----|
| Reranker / LLM integration in core | Belongs in user-space; we provide the storage primitive |
| Native cloud (multi-AZ, K8s operator) | Conflicts with local-first thesis; tracked separately |
| Built-in embedding model | Embedding is a model concern, not a storage concern |
| Replacing PostgreSQL | We are a vector + graph + columnar **niche**, not a generalist OLTP/OLAP engine |
| iOS/Android UI SDK | Mobile bindings exist (UniFFI) but UI components are app-developer territory |

---

## Cadence

- **Patch releases (v1.18.x):** as needed, no roadmap commitment
- **Minor releases (v1.14 → v1.18):** planned with a public milestone and OKRs; in practice the v1.14 → v1.18 line shipped at a weekly-to-biweekly cadence (2026-04-30 → 2026-06-07) as remediation and parity waves landed — expect minors to keep shipping when the milestone is done, not on a fixed calendar
- **Major release (v2.0):** no committed timeline. Will only happen if we need a breaking API change. The `#[non_exhaustive]` discipline on public enums keeps this option open.

## How this roadmap is governed

- **Milestone review** at each minor release (lessons learned, scope adjustment for next minor)
- **Public discussion** of any addition to Horizon 1 in a GitHub Discussion before commit
- **Labels:** `roadmap` = tracked but not committed; **a milestone** = committed
- **No surprise features** — everything user-facing in a release was on the milestone first

## Who decides

VelesDB is currently maintained by a single founder (Wiscale, France). Decisions follow this hierarchy:

1. **Hard constraints first:** recall ≥ 0.95, no breaking changes within a minor, no production unwrap, no degradation of the 450µs end-to-end claim
2. **Public discussion** for new features (GitHub Discussions or labeled issue)
3. **Founder call** for architecture / scope / cadence
4. **Community PRs welcomed** for any item on this roadmap — see `CONTRIBUTING.md`

When VelesDB adds a co-founder or technical advisor, this section will be updated.

---

## Related documents

- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to contribute, quality gates
- [`QUALITY_BAR.md`](QUALITY_BAR.md) — explicit metrics gates we will not ship below
- [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md) — current technical limitations with tracking issues
- [`CHANGELOG.md`](CHANGELOG.md) — what shipped in each release
