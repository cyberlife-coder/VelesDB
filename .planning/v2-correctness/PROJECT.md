# VelesDB Core — v2 Implementation Truth & Correctness

## What This Is

A correctness milestone for VelesDB Core. Every subsystem must do exactly what it claims to do — no fake implementations, no silent no-ops, no wrong formulas. If a feature isn't ready, it must error or be removed, never silently produce wrong results.

## Core Value

**Trust:** Every line of code does what its API contract promises. Users can rely on VelesDB producing correct results across all configurations (metrics, fusion strategies, quantization modes, graph traversal, VelesQL queries).

## Origin

All findings come from the Devil's Advocate Code Review (see `DEVIL_ADVOCATE_FINDINGS.md`). 40 issues identified across 2 audit phases: 5 critical, 12 bugs, 17 design, 3 minor + 3 CI.

## Requirements

### v1 — Must Fix (Correctness)

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| FIX-01 | C-01,C-02 | 🚨 | GPU methods that are CPU: remove fake GPU or implement real shaders |
| FIX-02 | C-03 | 🚨 | GPU brute force ignores distance metric — must respect index config |
| FIX-03 | C-04 | 🚨 | RRF formula is wrong — implement real rank-based RRF |
| FIX-04 | B-01 | 🐛 | NaN/Infinity vectors pass through VelesQL extraction |
| FIX-05 | B-02 | 🐛 | ORDER BY property paths silent no-op — error or implement |
| FIX-06 | B-03 | 🐛 | Weighted fusion = Average — add real weight config or remove |
| FIX-07 | B-04 | 🐛 | DualPrecision default search doesn't use quantized distances |
| FIX-08 | B-05 | 🐛 | BFS visited_overflow clears visited set — causes duplicates |
| FIX-09 | B-06 | 🐛 | cosine_similarity_quantized full dequantization for norm |
| FIX-10 | D-09 | ⚠️ | Fusion params silently default to 0.0 on parse error |

### v2 — Should Fix (Performance & Design)

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| OPT-01 | D-01 | ⚠️ | ColumnStore dual deletion tracking — unify to one structure |
| OPT-02 | D-02 | ⚠️ | HNSW layer lock per-iteration contention |
| OPT-03 | D-03 | ⚠️ | CART Node4 dead code + leaf splitting absent |
| OPT-04 | D-04 | ⚠️ | Over-fetch factor hardcoded at 10x |
| OPT-05 | D-05 | ⚠️ | WAL no per-entry CRC |
| OPT-06 | D-06 | ⚠️ | LogPayloadStorage flush per store |
| OPT-07 | D-07 | ⚠️ | should_create_snapshot takes write lock |
| OPT-08 | D-08 | ⚠️ | Two QuantizedVector types with same name |

### v3 — Ecosystem Correctness (Server, WASM, SDK, Integrations, CI)

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| ECO-01 | S-01 | 🚨 | Server: No authentication/authorization |
| ECO-02 | S-02 | 🐛 | Server: Handlers block async runtime |
| ECO-03 | S-03 | ⚠️ | Server: GraphService disconnected from core |
| ECO-04 | S-04 | ⚠️ | Server: No rate limiting |
| ECO-05 | W-01 | 🐛 | WASM: insert_batch ignores storage mode |
| ECO-06 | W-02 | 🐛 | WASM: hybrid_search silent fallback |
| ECO-07 | W-03 | ⚠️ | WASM: No ANN index (brute force only) |
| ECO-08 | T-01 | 🐛 | SDK: search() doesn't unwrap response |
| ECO-09 | T-02 | 🐛 | SDK: listCollections type mismatch |
| ECO-10 | T-03 | ⚠️ | SDK: query() ignores collection param |
| ECO-11 | I-01 | 🐛 | Integr: ID counter resets per instance |
| ECO-12 | I-02 | 🐛 | Integr: velesql() missing validation |
| ECO-13 | I-03 | ⚠️ | Integr: 80% code duplication |
| ECO-14 | CI-01 | ⚠️ | CI: PR validation disabled |
| ECO-15 | CI-02 | ⚠️ | CI: Security audit never fails |
| ECO-16 | CI-03 | ⚠️ | CI: cargo deny not in pipeline |
| ECO-17 | CI-04 | ⚠️ | CI: Python tests silently swallowed |

### Out of Scope

- New features (new query types, new distance metrics)
- Breaking API changes to public VelesQL grammar
- Full HNSW ANN index for WASM (design spike only — documented limitation)

## Constraints

- **TDD:** Test BEFORE code for every fix
- **Backward compatible:** VelesQL grammar unchanged, public API stable
- **Quality gates:** `cargo fmt + clippy + deny + test` must pass
- **Performance:** No regressions on HNSW search benchmarks
- **Atomic commits:** One finding per commit, clear commit messages

---
*Created from Devil's Advocate Review findings*
