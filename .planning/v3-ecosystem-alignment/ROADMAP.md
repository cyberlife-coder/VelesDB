# VelesDB v3 — Ecosystem Alignment Roadmap

**Version:** 1.0  
**Created:** 2025-02-08  
**Milestone:** v3-ecosystem-alignment (2 of 2)  
**Previous Milestone:** v2-core-trust  
**Total Phases:** 5  
**Findings covered:** 22/47 (ecosystem findings)  
**Source:** Devil's Advocate Code Review (`DEVIL_ADVOCATE_FINDINGS.md`)

---

## Architectural Principle

> **velesdb-core = single source of truth.**  
> All external components are bindings/wrappers. Zero reimplemented logic.  
> If a feature doesn't exist in core, it doesn't exist anywhere.

---

## Phase 1: WASM Rebinding

**Goal:** Replace the WASM VectorStore reimplementation with proper bindings to velesdb-core.

**Findings addressed:** BEG-01, BEG-05, BEG-06, W-01, W-02, W-03

### Tasks

1. **Audit** — Map every WASM function to its velesdb-core equivalent; identify gaps
2. **Core API surface** — Expose any missing core functions needed by WASM (e.g., in-memory collection without persistence)
3. **Replace VectorStore** — Bind to core's Collection/Index/Search instead of flat Vec arrays
4. **Replace GraphStore** — Bind to core's EdgeStore instead of separate implementation
5. **Fix clippy suppression** — Remove blanket pedantic/nursery allows; keep only wasm_bindgen-necessary ones
6. **Tests:** WASM search matches core search for same data; all storage modes work; graph traversal matches core

### Success Criteria

- WASM VectorStore delegates to velesdb-core (no own search/insert logic)
- Graph operations use core's EdgeStore
- Clippy pedantic enabled (with targeted allows only)
- Search results identical between WASM and core for same data

---

## Phase 2: Server Binding & Security

**Goal:** Server becomes a thin HTTP layer over velesdb-core. Add auth and runtime safety.

**Findings addressed:** S-01, S-02, S-03, S-04, BEG-05

### Tasks

1. **Authentication** — API key middleware (`Authorization: Bearer` + `VELESDB_API_KEY` env var)
2. **Unify graph** — Replace `GraphService` in-memory store with core's EdgeStore
3. **`spawn_blocking`** — Wrap CPU-intensive handlers (search, query, traversal)
4. **Rate limiting** — `tower-governor` or equivalent, configurable per IP
5. **Remove duplicate BFS/DFS** — Server calls core's traversal, not its own implementation
6. **Tests:** Auth rejection; concurrent search; rate limit 429; graph results match core

### Success Criteria

- Unauthenticated → 401
- Zero reimplemented graph/traversal logic in server
- CPU handlers don't block Tokio
- Rate limiting active

---

## Phase 2.1: Server Documentation Update

**Goal:** Bring server crate README and lib.rs docs up to date with all 26 API routes, auth, rate limiting, CORS, graph, indexes.

**Inserted:** 2025-02-10 — Documentation audit after v3-02 revealed significant gaps

### Tasks

1. **README.md** — Add missing sections (Auth, Rate Limiting, Graph, Indexes, Match, Flush/Empty, Explain), fix incorrect Points examples, update Configuration table
2. **lib.rs** — Update OpenAPI version, enrich crate-level doc comment
3. **CHANGELOG.md** — Reflect v3-02 changes

### Success Criteria

- [ ] All 26 API routes documented with curl examples
- [ ] All env vars documented (VELESDB_API_KEY, VELESDB_RATE_LIMIT, VELESDB_RATE_BURST, VELESDB_CORS_ORIGIN)
- [ ] Incorrect Points API examples fixed
- [ ] Authentication section present
- [ ] Graph API section present
- [ ] lib.rs OpenAPI version matches crate version

---

## Phase 2.2: Server Hardening — Validation, Consistency, Error Hygiene

**Goal:** Address code review findings: harmonize handler patterns, add input validation, sanitize 500 error messages.

**Inserted:** 2025-02-10 — Code review identified validation gaps, inconsistent patterns, and exposed internals in error messages

### Tasks

1. **Consistency** — Extract `get_collection_or_404()` to shared helpers module, replace duplicated match blocks in all handlers
2. **Input Validation** — Add server-side bounds: `top_k` (1..10000), `dimension` > 0, `vector.len() == collection.dimension`, query strings non-empty
3. **Error Hygiene** — Replace `"Task panicked: {e}"` in 500 responses with generic client message; log internal details server-side via `tracing::error!`

### Success Criteria

- [ ] Single `get_collection_or_404()` used by all handlers (zero duplicated match blocks)
- [ ] Invalid `top_k`, `dimension`, or vector length returns 400 with clear message
- [ ] No internal error details leaked in HTTP 500 responses
- [ ] All existing tests still pass
- [ ] Clippy clean (`-D warnings`)

---

## Phase 3: TypeScript SDK Fixes

**Goal:** SDK correctly maps server responses and handles concurrent init.

**Findings addressed:** T-01, T-02, T-03, BEG-07

### Tasks

1. **`search()` response** — Extract `.results` (like `textSearch`/`hybridSearch` already do)
2. **`listCollections()`** — Unwrap `{ collections: [...] }` wrapper
3. **`query()` collection param** — Remove or use in URL
4. **`init()` race guard** — Promise-based lock for concurrent callers
5. **Tests:** search returns array; listCollections returns Collection[]; concurrent init safe

### Success Criteria

- `search()` → `SearchResult[]`
- `listCollections()` → `Collection[]`
- No init race condition

---

## Phase 4: Python Integrations

**Goal:** LangChain and LlamaIndex integrations are correct, secure, and share common code.

**Findings addressed:** I-01, I-02, I-03, BEG-02, BEG-03, BEG-04

### Tasks

1. **Extract `velesdb-python-common`** — Shared `_stable_hash_id`, result parsing, payload building
2. **Fix `storage_mode`** — Pass to `create_collection()` in both integrations
3. **Fix ID generation** — Hash-based IDs, no sequential counter
4. **Fix `velesql()` validation** — Add `validate_query()` in LlamaIndex
5. **Factor `add_texts_bulk`** — DRY with `add_texts` via shared internal method
6. **Fix security module** — `validate_path` must sandbox to `base_directory`
7. **Tests:** storage_mode SQ8 works; no ID collision; velesql injection blocked; path sandboxing

### Success Criteria

- Shared Python module used by both integrations
- `storage_mode` actually applied
- ID collision impossible
- Path traversal blocked

---

## Phase 5: GPU Extras + Ecosystem CI

**Goal:** Remaining GPU metrics and full ecosystem test coverage in CI.

**Findings addressed:** I-04, CI-04

### Tasks

1. **Hamming/Jaccard GPU shaders** — WGSL implementation + pipeline wiring
2. **Python CI** — Both LangChain and LlamaIndex tests run and block on failure
3. **SDK CI** — `npm test` in CI for TypeScript SDK
4. **WASM CI** — `wasm-pack test` in CI
5. **Tests:** All GPU metrics have GPU+CPU equivalence tests; ecosystem CI green

### Success Criteria

- All 5 distance metrics have GPU pipelines
- Python, TS, WASM tests all run in CI and block on failure

---

## Progress Tracker

| Phase | Status | Scope | Priority |
|-------|--------|-------|----------|
| 1 - WASM Rebinding | ⬜ Pending | BEG-01,05,06, W-01→03 | 🚨 Architecture |
| 2 - Server Binding | ✅ Complete | S-01→04, BEG-05 | 🚨 Security |
| 2.1 - Server Docs | ✅ Complete | Documentation gaps | 📚 Documentation |
| 2.2 - Server Hardening | ✅ Complete | Validation, consistency, error hygiene | 🛡️ Quality |
| 3 - SDK Fixes | ⬜ Pending | T-01→03, BEG-07 | 🐛 Contracts |
| 4 - Python Integrations | ⬜ Pending | I-01→03, BEG-02→04 | 🐛 Contracts |
| 5 - GPU + Ecosystem CI | ⬜ Pending | I-04, CI-04 | ⚠️ Polish |

**Execution:** `1 → 2 → 3 → 4 → 5`  
**Findings covered:** 22/47 (ecosystem)

---

## Quality Gates (per phase)

```powershell
# Core (always)
cargo fmt --all --check
cargo clippy -- -D warnings
cargo deny check
cargo test --workspace
cargo build --release

# Ecosystem (phase-specific)
wasm-pack test --headless --chrome    # WASM
npm test                               # SDK
pytest                                 # Python integrations
```

---
*Milestone v3 — Ecosystem Alignment. Depends on v2-core-trust completion.*
