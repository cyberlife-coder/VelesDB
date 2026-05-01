# VelesDB Roadmap

This roadmap commits to **what we are building**, **why**, and **when**. It is updated each minor release and synced with the GitHub Milestones.

It is intentionally narrow. Items not on this roadmap are tracked as `roadmap` issues but **not committed** until they reach a milestone here.

> **Last updated:** 2026-05-01 — covers v1.14.2 (current) → v1.16.0 horizon.

---

## Horizon 1 — Next 3 months (v1.14.x → v1.15.0)

### Theme: Ecosystem credibility & adoption signals

VelesDB v1.14.x has shipped the ecosystem-credibility foundations: the Python RAG framework trio (LangChain + LlamaIndex + Haystack) is complete, MSRV is honestly aligned with the actual SIMD path, and the release pipeline now keeps 18 manifests/snippets/Dockerfile labels in lock-step. The next milestone moves the project from "ecosystem-credible" to "commercially adoptable" via Python DataFrame ergonomics, CBO calibration, and the SearchOptions builder refactor.

**Milestone:** [v1.15.0](https://github.com/cyberlife-coder/VelesDB/milestones)

| # | Item | Success criterion | Status |
|---|------|------------------|--------|
| 1 | [Haystack 2.x DocumentStore integration (#349)](https://github.com/cyberlife-coder/VelesDB/issues/349) | At least one BDD test passing in CI; published in `integrations/haystack/`; first community contribution merged | ✅ Shipped in v1.14.0 / v1.14.1 — PR [#672](https://github.com/cyberlife-coder/VelesDB/pull/672) by [@CrepuscularIRIS](https://github.com/CrepuscularIRIS); `pip install haystack-velesdb` live on PyPI |
| 2 | [Onboarding time-to-first-search < 5 min (#379)](https://github.com/cyberlife-coder/VelesDB/issues/379) | Measured on clean Ubuntu/macOS/Windows; documented in `docs/quickstart/timing-results.md`; verified via reproducible Docker harness | ✅ Shipped in v1.13.7 (Phase 6) — median across 4 paths under 26 s; [`scripts/dx-timing/run_all.sh`](scripts/dx-timing/run_all.sh) reproduces it |
| 3 | [CBO calibration loop (#469)](https://github.com/cyberlife-coder/VelesDB/issues/469) | `COST_UNIT_TO_MS` recalibrated from real query timings; method documented in `BENCHMARKS.md`; removes `KNOWN_LIMITATIONS.md #1` | Open (slated for v1.15.0) |
| 4 | [Python DataFrame + Polars integration (#429)](https://github.com/cyberlife-coder/VelesDB/issues/429) | `upsert_dataframe(df)` + `search().to_polars()` round-trip; one notebook in `examples/python/` | Open (slated for v1.15.0) |
| 5 | [PyO3 SearchOptions builder (#717)](https://github.com/cyberlife-coder/VelesDB/issues/717) | Replace the wide-kwarg `Collection.search` signature with a builder pattern + deprecation cycle; remove the `clippy::too_many_arguments` allow-list | Open (slated for v1.15.0/v2.0.0) |

**Already shipped in v1.14.x:** MSRV bump 1.83 → 1.89 (#714), Dockerfile auto-sync (#715), full Python RAG framework trio (Haystack via #672), doc consistency sweep (#722), `haystack-velesdb` PyPI publishing (#723).

---

## Horizon 2 — 3 to 6 months (v1.15.0)

### Theme: Performance narrative & SDK parity

By v1.15 we want a single sentence pitch: *"VelesDB is the only embedded vector + graph + columnar engine, faster than competitors on full-path latency, with first-class SDKs in 4 languages."*

**Tentative scope:**

| # | Item | Why |
|---|------|-----|
| 1 | [HNSW <30µs index-only target (#377)](https://github.com/cyberlife-coder/VelesDB/issues/377) | Push the index-only micro-bench from 55µs to <30µs to widen the headroom on the 450µs full-path number |
| 2 | [SDK parity: TypeScript/LangChain/LlamaIndex (#380)](https://github.com/cyberlife-coder/VelesDB/issues/380) | Close the cross-language gap so any framework user gets the same API surface |
| 3 | **Reproducible head-to-head benchmark vs Qdrant + Chroma + pgvector** (Docker Compose) | Pre-seed audit P0: turn marketing claims into proven numbers |
| 4 | **External `unsafe` audit** (SIMD module, Cure53 / independent Rust safety expert) | Required for "data sovereignty" enterprise positioning |

These items need budget commitment (audit ~5-15k€) and are conditional on funding closing.

---

## Horizon 3 — 6 to 12 months (v1.16.0+)

### Theme: Enterprise feature gate & multi-tenancy

By v1.16 we want VelesDB to be deployable in production at small-team scale (5-50 user services) with credible operational primitives. Most of these features will live in the **`velesdb-premium`** companion crate (separate repo) under a commercial license, with the OSS core remaining feature-complete for single-tenant local-first use cases.

**Tentative scope:**

- **Concurrent WAL writer** with batching (today: single-writer mutex)
- **Multi-tenancy / namespacing** (today: per-database isolation only)
- **RBAC** (Role-Based Access Control) — premium
- **Distributed replication** (Raft) — premium, long horizon
- **Query result caching with auth tags** — premium

The deferred [Hardware Accelerator backlog (#689)](https://github.com/cyberlife-coder/VelesDB/issues/689) (CUDA, AVX-512-VNNI, SVE, FP16, etc.) will be re-evaluated at this horizon based on customer requests.

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

- **Patch releases (v1.13.x):** as needed, 1-2x per month, no roadmap commitment
- **Minor releases (v1.14, v1.15, v1.16):** ~3 months apart, each with a public milestone and OKRs
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
