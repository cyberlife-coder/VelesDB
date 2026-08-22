# VelesDB Roadmap

This roadmap commits to **what we are building**, **why**, and **when**. It is updated each minor release and synced with the GitHub Milestones.

It is intentionally narrow. Items not on this roadmap are tracked as `roadmap` issues but **not committed** until they reach a milestone here.

> **Last updated:** 2026-08-17 — covers v5.2.0 (current) — the workspace manifest version; the latest published packages are 5.0.0, `velesdb-memory` 0.12.0. The v1.x-era horizon framing is retired: everything those horizons shipped is compressed into the Delivered table, and the next commitments are derived from the repo's own registers ([`CHANGELOG.md`](CHANGELOG.md) [Unreleased], [`docs/CORE_WIRING_DEBT.md`](docs/CORE_WIRING_DEBT.md), [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md)).

---

## Delivered — the v1.x horizons (v1.13.7 → v2.0.0)

One line per item; the [`CHANGELOG.md`](CHANGELOG.md) is the per-release record. Items the horizons committed but never shipped are marked and re-triaged (→ Next, → Later, or back to `roadmap`-issue status).

| Item | Outcome | Link |
|---|---|---|
| Haystack 2.x DocumentStore ([#349](https://github.com/cyberlife-coder/VelesDB/issues/349)) | `haystack-velesdb` live on PyPI; first community contribution merged (v1.14.0/v1.14.1) | PR [#672](https://github.com/cyberlife-coder/VelesDB/pull/672) by [@CrepuscularIRIS](https://github.com/CrepuscularIRIS) |
| Onboarding time-to-first-search < 5 min ([#379](https://github.com/cyberlife-coder/VelesDB/issues/379)) | median under 26 s across 4 paths (v1.13.7), reproducible harness | [`scripts/dx-timing/run_all.sh`](scripts/dx-timing/run_all.sh) |
| CBO calibration Phase 2 ([#469](https://github.com/cyberlife-coder/VelesDB/issues/469)) | ⚠️ empirical EMA in `EXPLAIN ANALYZE` (v1.15.0); the full `COST_UNIT_TO_MS` pin never landed → **P3 below** | PR [#784](https://github.com/cyberlife-coder/VelesDB/pull/784) |
| Python DataFrame + Polars ([#429](https://github.com/cyberlife-coder/VelesDB/issues/429)) | `upsert_from_dataframe` (pandas/polars auto-detected) + `to_dataframe(backend="polars")` round-trip | [#429](https://github.com/cyberlife-coder/VelesDB/issues/429) |
| PyO3 `SearchOptions` builder ([#717](https://github.com/cyberlife-coder/VelesDB/issues/717)) | fluent builder replaces the wide-kwarg `search` signature (v1.15.0) | PR [#761](https://github.com/cyberlife-coder/VelesDB/pull/761) |
| ACT-R Phase 1 procedural learning | procedural-memory module (v1.15.0) | PR [#780](https://github.com/cyberlife-coder/VelesDB/pull/780) |
| Python auto-dimension | vector dimension inferred from first upsert (v1.15.0) | PR [#778](https://github.com/cyberlife-coder/VelesDB/pull/778) |
| `IN` filter O(log n) | binary-search filter path (v1.15.0) | PR [#765](https://github.com/cyberlife-coder/VelesDB/pull/765) |
| HNSW <30µs index-only ([#377](https://github.com/cyberlife-coder/VelesDB/issues/377)) | `ANALYZE`-triggered in-place node reorder; 10K-probe recall@10 off-by-one fixed (v1.15.0) | PR [#785](https://github.com/cyberlife-coder/VelesDB/pull/785) |
| SDK parity: TS/LangChain/LlamaIndex ([#380](https://github.com/cyberlife-coder/VelesDB/issues/380)) | TS REST backend gains `sparseIndexName` + RSF weights (v1.15.0) | PR [#779](https://github.com/cyberlife-coder/VelesDB/pull/779) |
| SIMD kernel coverage | AVX-512 / AVX2 / NEON f32·f16 kernels for cosine·dot·euclidean (v1.13.x) | [`docs/reference/NATIVE_HNSW.md`](docs/reference/NATIVE_HNSW.md) |
| `audit-2026q2` security hardening | 9-PR wave: on-disk validation, allocation caps, parser DoS bounds, rate limiter (v1.16.0) | PRs [#908](https://github.com/cyberlife-coder/VelesDB/pull/908)–[#916](https://github.com/cyberlife-coder/VelesDB/pull/916) |
| First-party embedding adapters | Python + TypeScript (v1.16.0) | PR [#917](https://github.com/cyberlife-coder/VelesDB/pull/917) |
| Typed Tauri guest-JS wrappers | 9 wrappers (v1.16.0) | PR [#928](https://github.com/cyberlife-coder/VelesDB/pull/928) |
| VelesQL parser error hints | did-you-mean suggestions (v1.17.0) | [#987](https://github.com/cyberlife-coder/VelesDB/pull/987) |
| Payload-WAL torn-tail recovery | crash recovery on the payload WAL (v1.17.0) | [#1011](https://github.com/cyberlife-coder/VelesDB/pull/1011) |
| Fusion / HNSW parameter validation | hybrid weight + `alpha` boundary validation (v1.17.0) | [#1013](https://github.com/cyberlife-coder/VelesDB/pull/1013), [#1015](https://github.com/cyberlife-coder/VelesDB/pull/1015) |
| HNSW probe-RNG contention removed | search-path contention fix (v1.17.0) | [#1001](https://github.com/cyberlife-coder/VelesDB/pull/1001) |
| Core-license artifact realignment | engine-embedding artifacts under VelesDB Core License 1.0 (v1.18.0) | [#1053](https://github.com/cyberlife-coder/VelesDB/pull/1053) |
| Python agent-memory bindings | TTL, snapshots, VelesQL bridges (v1.18.0) | [#1045](https://github.com/cyberlife-coder/VelesDB/pull/1045) |
| Tauri agent-memory commands | parity with Python bindings (v1.18.0) | [#1046](https://github.com/cyberlife-coder/VelesDB/pull/1046) |
| Agent-memory TTL & expiry hardening | (v1.18.0) | [#1040](https://github.com/cyberlife-coder/VelesDB/pull/1040)/[#1043](https://github.com/cyberlife-coder/VelesDB/pull/1043) |
| TS procedural recall | fixed via required `embedding` (v1.18.0) | [#1039](https://github.com/cyberlife-coder/VelesDB/pull/1039) |
| Horizon-4 flagship queue | graph `relate()`, durable TTL on every read path, PQ/RaBitQ wired across restarts, persisted-HNSW reload at open — landed in v2.0.0, there never was a v1.19 | [`CHANGELOG.md`](CHANGELOG.md) `[2.0.0]` |
| Head-to-head benchmark vs Qdrant + Chroma + pgvector | ⚠️ only the pgvector leg is reproducible (`benchmarks/` Docker Compose); the Qdrant + Chroma legs never landed — back to `roadmap`-issue status, not committed | [`benchmarks/`](benchmarks/) |
| External `unsafe` audit (SIMD module) | ❌ never funded (~5-15 k€) — remains a `roadmap` issue, not committed | — |
| `velesdb-migrate` rework decision | ❌ decision never made → **Later below** | [`ARCHITECTURE.md`](ARCHITECTURE.md) crate table |
| Ship the pending major correctly (P1) | v5.0.0 tagged 2026-08-10 carrying the `load_working_context` envelope break; `@wiscale/velesdb-wasm` floor raised to `^5.0.0` in the TS SDK and the `velesdb` floor to `>=5.0.0` in `langgraph-velesdb` — the runtime skew guards are nets again, no longer the fence | [`CHANGELOG.md`](CHANGELOG.md) `[5.0.0]` |

---

## Next (committed)

Two items, in priority order. Each has a register in the repository as its source of truth; done means that register says so. (P1 — ship the pending major correctly — was delivered as v5.0.0; see the Delivered table.)

### P2 — Pay down the core wiring debt

[`docs/CORE_WIRING_DEBT.md`](docs/CORE_WIRING_DEBT.md) registers subsystems that exist in `velesdb-core` but are not wired to the runtime; each entry names its target outcome. Three remain open:

| Entry | State | Target outcome |
|---|---|---|
| 1 — `WalBatchConfig` / `WalBatcher` | code exists, **zero call sites**, now `pub(crate)` (off the public surface); TOML parsed and ignored | execute the declared premium transfer — or drop the dormant `enabled` field in the 5.0.0 break |
| 3 — `deferred_indexing` / `async_index_builder` | runtime-wired; no TOML/create-time surface | the "Streaming Ingestion Configuration" RFC |
| 4 — `SearchConfig` global defaults | hard-coded local defaults shadow the runtime config | consolidate the fallback chain through one helper |

Success criterion: the entries are closed in that registry.

### P3 — Close the oldest carried honesty debt: empirical `COST_UNIT_TO_MS` pin ([#469](https://github.com/cyberlife-coder/VelesDB/issues/469))

Carried since v1.15 (Phase 2 shipped only the `EXPLAIN ANALYZE` EMA — see Delivered). [`docs/reference/KNOWN_LIMITATIONS.md`](docs/reference/KNOWN_LIMITATIONS.md) §1 still stands: `COST_UNIT_TO_MS = 0.001` in `node_stats.rs` awaits empirical calibration, so pre- and post-`ANALYZE` costs sit ~22× apart in magnitude. Success criterion: pin the constant from a micro-benchmark of a known plan shape on reference hardware, document the method in [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md) (which does not describe it yet), and delete `KNOWN_LIMITATIONS.md` §1.

---

## Later (tracked, not committed)

No dates. These graduate to **Next** only through a milestone.

- **FP16/BF16 compute paths, then IVF / DiskANN-style exploration** — the open items of [`docs/ANN_SOTA_AUDIT.md`](docs/ANN_SOTA_AUDIT.md): FP16/BF16 kernels first, then a partitioned coarse stage (IVF/IMI) and a disk-backed graph search path, both explicitly absent today.
- **Native installers** — [`docs/planning/NATIVE_INSTALLERS.md`](docs/planning/NATIVE_INSTALLERS.md) is an open decision note: five options on a strictly increasing cost curve (`.mcpb` → Homebrew/winget → signed `.pkg`/`.msi` → packaged GUI), and the audience question it poses must be answered before any of them is built.
- **Extract `velesdb-migrate` to its own repo** — teased in the [`ARCHITECTURE.md`](ARCHITECTURE.md) crate table ("strategic candidate to extract to a separate repo"); the keep / extract / archive criteria from the v1.x roadmap (download counts, stars, open issues) still apply.

---

## velesdb-memory line (independent 0.x cadence)

`velesdb-memory` (0.13.0) versions and releases on its own 0.x cadence, decoupled from the workspace majors.

- **LoCoMo temporal-decomposition follow-up** — [`docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md`](docs/planning/LOCOMO_TEMPORAL_DECOMP_RESEARCH.md) concluded: dated recall's +33.6pp temporal lift is real and ironclad; the scaffold's marginal gain is unproven at n=321 and its feared single-hop cost is not real. The honest follow-up it scopes is a *cost*-framed routing chapter (skip the scaffold's CoT tokens where they don't move accuracy) — and it needs its own justification before it runs.
- **Context-compiler evolution** — the deterministic compiler (`compile_context` and friends) evolves per `crates/velesdb-memory/CHANGELOG.md`.

Measurement discipline for both: [`crates/velesdb-memory/BENCHMARK.md`](crates/velesdb-memory/BENCHMARK.md) — generation-free retrieval metrics first, paired statistics (McNemar, cluster bootstrap) behind every end-to-end claim.
