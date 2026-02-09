# VelesDB v3 — Ecosystem Alignment

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components (server, WASM, SDK, integrations) are bindings/wrappers.  
> Zero reimplemented logic. Zero duplicated code.  
> If a feature doesn't exist in core, it doesn't exist anywhere.

## What This Is

A milestone to **align the entire VelesDB ecosystem** with velesdb-core. Every external component must become a proper binding/wrapper with zero reimplemented logic. This is NOT about fixing bugs in reimplemented code — it's about **deleting that code and replacing it with bindings**.

## Prerequisite

- ✅ **v2-core-trust** — completed 2026-02-08 (23/23 findings resolved)
- ✅ **v4-verify-promise** — completed 2026-02-09 (13/13 requirements, README honest mirror)

Core is trustworthy and documented. Ready to build bindings.

## Core Value

**Consistency:** A search in WASM returns the same result as a search in the server, the SDK, and the Python integration — because they all call the same velesdb-core code path.

## Origin

22 findings from the Devil's Advocate Code Review that affect external components. See `DEVIL_ADVOCATE_FINDINGS.md`.

## Requirements

### v1 — Must Fix (Binding Architecture)

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| BIND-01 | BEG-01 | 🚨 | WASM VectorStore is a full reimplementation → replace with core binding |
| BIND-02 | BEG-05 | 🚨 | 3 parallel BFS/DFS → server and WASM must use core's traversal |
| BIND-03 | S-03 | ⚠️ | Server GraphService disconnected → bind to core EdgeStore |
| BIND-04 | BEG-06 | ⚠️ | 16 clippy allows in WASM → proper quality checks |

### v2 — Must Fix (Contract Correctness)

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| API-01 | S-01 | 🚨 | Server: No authentication |
| API-02 | S-02 | 🐛 | Server: Handlers block async runtime |
| API-03 | S-04 | ⚠️ | Server: No rate limiting |
| API-04 | T-01 | 🐛 | SDK: search() doesn't unwrap response |
| API-05 | T-02 | 🐛 | SDK: listCollections type mismatch |
| API-06 | T-03 | ⚠️ | SDK: query() ignores collection param |
| API-07 | BEG-07 | 🐛 | SDK: init() race condition |

### v3 — Must Fix (Integration Quality)

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| INT-01 | I-01 | 🐛 | ID counter resets per instance |
| INT-02 | I-02 | 🐛 | velesql() missing validation |
| INT-03 | I-03 | ⚠️ | 80% code duplication LangChain/LlamaIndex |
| INT-04 | BEG-02 | 🐛 | storage_mode dead code (never passed) |
| INT-05 | BEG-03 | ⚠️ | add_texts_bulk pure copy-paste |
| INT-06 | BEG-04 | ⚠️ | Security validation is theater |

### v4 — Nice to Have

| ID | Finding | Severity | Description |
|----|---------|----------|-------------|
| GPU-01 | I-04 | ⚠️ | Hamming/Jaccard GPU shaders |
| WASM-01 | W-01→03 | 🐛 | WASM bugs (will be fixed by rebinding) |

### Out of Scope

- New features
- Breaking changes to VelesQL grammar
- Mobile/Tauri plugin rework

## Constraints

- ~~**Core must be v2-complete** before starting~~ ✅ v2 + v4 complete
- **TDD:** Test BEFORE code
- **Zero reimplementation:** If WASM needs a feature, add it to core first
- **Quality gates:** All `local-ci.ps1` checks + ecosystem-specific tests
- **Backward compatible SDK API:** Same function signatures, correct behavior

---
*Milestone v3 — Ecosystem Alignment. Prerequisites met: v2-core-trust ✅, v4-verify-promise ✅.*
