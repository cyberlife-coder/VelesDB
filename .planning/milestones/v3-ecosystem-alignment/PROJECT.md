# VelesDB v3 — Ecosystem Alignment (Expanded)

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.  
> If a feature doesn't exist in core, it doesn't exist anywhere.

## What This Is

A milestone to **align the entire VelesDB ecosystem** with velesdb-core. Every external component must become a proper binding/wrapper with zero reimplemented logic. This includes fixing Devil's Advocate findings AND updating all demos, examples, and documentation to reflect the v4-verify-promise changes.

**This is NOT about fixing bugs in reimplemented code — it's about deleting that code and replacing it with bindings.** Then ensuring every user-facing artifact (demos, examples, READMEs) works end-to-end.

## Prerequisites — ✅ All Met

- ✅ **v2-core-trust** — completed 2026-02-08 (23/23 findings resolved)
- ✅ **v4-verify-promise** — completed 2026-02-09 (13/13 requirements, ~176 new tests)

Core is trustworthy, VelesQL execution is complete, README is an honest mirror. Ready to build bindings and align ecosystem.

## Core Value

**Consistency:** A search in WASM returns the same result as a search in the server, the SDK, and the Python integration — because they all call the same velesdb-core code path.

**User Experience:** Every demo compiles, every example runs, every integration test passes. A developer cloning VelesDB gets a working ecosystem on the first try.

## Origin

- **22 findings** from Devil's Advocate Code Review (ecosystem scope)
- **8 new findings** from v3 ecosystem audit (demos, examples, Tauri, versions)
- **Total: 30 findings** across 10 components

---

## Ecosystem Inventory

| Component | Path | Type | Uses Core? | Health |
|-----------|------|------|------------|--------|
| `velesdb-wasm` | `crates/velesdb-wasm/` | WASM bindings | ❌ Full reimplementation (21 files) | 🔴 Critical |
| `velesdb-server` | `crates/velesdb-server/` | HTTP API | ✅ Partial (graph disconnected) | 🟡 Needs auth/async |
| `velesdb-python` | `crates/velesdb-python/` | PyO3 SDK | ✅ Uses `CoreDatabase` | 🟢 Good binding |
| `@wiscale/velesdb-sdk` | `sdks/typescript/` | TS SDK | Via REST or WASM backend | 🟡 3 bugs |
| `langchain-velesdb` | `integrations/langchain/` | LangChain | Via `velesdb` (PyO3) | 🟡 80% duplication |
| `llama-index-vector-stores-velesdb` | `integrations/llamaindex/` | LlamaIndex | Via `velesdb` (PyO3) | 🟡 80% duplication |
| `tauri-plugin-velesdb` | `crates/tauri-plugin-velesdb/` | Tauri plugin | ✅ Uses core | 🟡 Not audited |
| `rag-pdf-demo` | `demos/rag-pdf-demo/` | Demo | Via httpx → server | 🟠 Stale APIs |
| `tauri-rag-app` | `demos/tauri-rag-app/` | Demo | Via tauri plugin | 🟠 Stale APIs |
| `examples/` | `examples/` | Examples | Mixed (core, REST, WASM) | 🟠 Likely stale |

---

## Requirements

### Tier 1 — Architecture (🚨 Must Fix First)

| ID | Source | Severity | Description |
|----|--------|----------|-------------|
| ECO-01 | BEG-01 | 🚨 | WASM VectorStore is a full reimplementation → replace with core binding |
| ECO-02 | BEG-05 | 🚨 | 3 parallel BFS/DFS → server and WASM must use core's traversal |
| ECO-03 | S-01 | 🚨 | Server: No authentication/authorization |
| ECO-04 | S-03 | ⚠️ | Server GraphService disconnected → bind to core EdgeStore |

### Tier 2 — Contract Correctness (🐛 Bugs)

| ID | Source | Severity | Description |
|----|--------|----------|-------------|
| ECO-05 | S-02 | � | Server: Handlers block async runtime (no spawn_blocking) |
| ECO-06 | W-01 | 🐛 | WASM insert_batch ignores storage mode |
| ECO-07 | W-02 | 🐛 | WASM hybrid_search silently drops text for non-Full |
| ECO-08 | T-01 | 🐛 | TS SDK: search() doesn't unwrap server response |
| ECO-09 | T-02 | 🐛 | TS SDK: listCollections type mismatch |
| ECO-10 | BEG-07 | 🐛 | TS SDK: init() TOCTOU race condition |
| ECO-11 | I-01 | 🐛 | LangChain: ID counter resets per instance |
| ECO-12 | I-02 | 🐛 | LlamaIndex: velesql() missing query validation |
| ECO-13 | BEG-02 | 🐛 | Python integrations: storage_mode dead code (never passed) |

### Tier 3 — Design Quality (⚠️ Structural)

| ID | Source | Severity | Description |
|----|--------|----------|-------------|
| ECO-14 | S-04 | ⚠️ | Server: No rate limiting |
| ECO-15 | T-03 | ⚠️ | TS SDK: query() ignores collection param |
| ECO-16 | BEG-06 | ⚠️ | WASM: 16 clippy allows suppress all quality |
| ECO-17 | W-03 | ⚠️ | WASM: No ANN index — brute force O(n) only |
| ECO-18 | I-03 | ⚠️ | 80% code duplication LangChain/LlamaIndex → extract common lib |
| ECO-19 | BEG-03 | ⚠️ | add_texts_bulk pure copy-paste of add_texts |
| ECO-20 | BEG-04 | ⚠️ | Security validation is theater (no path sandboxing) |
| ECO-21 | I-04 | ⚠️ | GPU: Missing Hamming/Jaccard shaders |
| ECO-22 | CI-04 | ⚠️ | CI: Python integration tests silently swallowed |

### Tier 4 — Ecosystem Freshness (📝 New from v3 Audit)

| ID | Source | Severity | Description |
|----|--------|----------|-------------|
| ECO-23 | Audit | 📝 | `rag-pdf-demo` uses httpx directly, not Python SDK — stale patterns |
| ECO-24 | Audit | 📝 | `tauri-rag-app` version 0.1.0 vs core 1.4.x — version mismatch |
| ECO-25 | Audit | 📝 | Python examples may use outdated VelesQL syntax (pre-v4 changes) |
| ECO-26 | Audit | 📝 | Rust examples may not compile against current core API |
| ECO-27 | Audit | � | WASM browser demo references old VectorStore API |
| ECO-28 | Audit | 📝 | `examples/README.md` API table incomplete (missing v4 endpoints) |
| ECO-29 | Audit | 📝 | `tauri-plugin-velesdb` not audited for core alignment |
| ECO-30 | Audit | 📝 | Version alignment needed across all components |

### Out of Scope

- New features not in Devil's Advocate findings
- Breaking changes to VelesQL grammar (v4 already complete)
- Performance optimization of core engine (already done in v1/v2)

---

## Phase Structure (7 Phases, Prioritized)

| Phase | Name | Requirements | Priority | Depends On |
|-------|------|-------------|----------|------------|
| **1** | WASM Rebinding | ECO-01,02,06,07,16,17 | 🚨 Architecture | — |
| **2** | Server Binding & Security | ECO-03,04,05,14 | 🚨 Security | — |
| **3** | Python Common + Integrations | ECO-11,12,13,18,19,20 | 🐛 DRY + Quality | — |
| **4** | TypeScript SDK Fixes | ECO-08,09,10,15 | 🐛 Contracts | Phase 2 |
| **5** | Demos & Examples Update | ECO-23,24,25,26,27,28,30 | 📝 User Experience | Phases 1-4 |
| **6** | Tauri Plugin Audit | ECO-29 | 🐛 Completeness | Phase 1 |
| **7** | GPU Extras + Ecosystem CI | ECO-21,22 | ⚠️ Polish | All |

**Execution order:** `1 → 2 → 3 → 4 → 5 → 6 → 7`  
(Phases 1-3 can run in parallel if independent contributors)

---

## Constraints

- ✅ **v2-core-trust + v4-verify-promise** complete
- **TDD:** Test BEFORE code for every fix
- **Zero reimplementation:** If WASM needs a feature, add it to core first
- **Quality gates:** All `local-ci.ps1` checks + ecosystem-specific tests (wasm-pack, npm test, pytest)
- **Backward compatible SDK API:** Same function signatures, correct behavior
- **Version alignment:** All components at same version after milestone
- **Every demo must run:** Clone → install → run must work end-to-end

---

## Competitor Analysis

| DB | SDK Pattern | Local-First? | VelesDB Differentiator |
|----|-------------|-------------|----------------------|
| **Qdrant** | Client-server (gRPC/REST). Separate SDK repos. | ❌ Server only | VelesDB: embedded WASM + desktop + mobile |
| **ChromaDB** | Embedded Python + REST client. JS is REST-only. | ✅ Python only | VelesDB: WASM in browser + Tauri desktop |
| **Weaviate** | Client-server. Multi-language SDKs. | ❌ Server only | VelesDB: no server required |

**VelesDB's unique edge:** True local-first across WASM, desktop (Tauri), and embedded (PyO3). But this only works if WASM actually uses core's logic. Phase 1 (WASM rebinding) is the foundation.

---
*Milestone v3 — Ecosystem Alignment (Expanded). 30 findings, 7 phases, 10 components.*
