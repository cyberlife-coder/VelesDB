# Phase 18: Documentation Code Audit - Research

**Researched:** 2026-03-08
**Domain:** Documentation accuracy, cross-crate API verification
**Confidence:** HIGH

## Summary

This phase audits ALL code snippets across the project's documentation files (READMEs, guides, specs, migration docs) against the real API surface in Rust, Python (PyO3), WASM, REST, and CLI. The research uncovered **28+ concrete mismatches** ranging from nonexistent methods and wrong class names to incorrect import paths and fabricated VelesQL syntax.

The most severe issues are in the root README.md (calling `db.search()` and `db.stream_insert()` which do not exist), SEARCH_MODES.md (using `FUSE BY` syntax not implemented in the grammar, and `collection.search_with_quality()` which is not a public Collection method), and docs/MIGRATION_v1.4_to_v1.5.md (using `VelesDB` class and `db.collection()` which do not exist).

**Primary recommendation:** Fix all mismatches in priority order: root README > crate READMEs > docs/guides > docs/reference. Each fix is a simple text replacement verified against the actual source code.

## Inventory of Documentation Files with Code Snippets

### Tier 1 -- High Visibility (README files)
| File | Code Blocks | Issues Found |
|------|-------------|--------------|
| `README.md` (root) | 71 | 6 critical |
| `crates/velesdb-core/README.md` | 11 | 1 minor |
| `crates/velesdb-server/README.md` | 10 | 0 |
| `crates/velesdb-cli/README.md` | 19 | 0 |
| `crates/velesdb-python/README.md` | 6 | 3 moderate |
| `crates/velesdb-wasm/README.md` | 11 | 2 moderate |
| `crates/velesdb-mobile/README.md` | 7 | 0 |
| `crates/velesdb-migrate/README.md` | 31 | 0 |
| `integrations/langchain/README.md` | 8 | 0 |
| `integrations/llamaindex/README.md` | 7 | 0 |

### Tier 2 -- User Guides
| File | Code Blocks | Issues Found |
|------|-------------|--------------|
| `docs/guides/SEARCH_MODES.md` | 35 | 8 critical |
| `docs/guides/USE_CASES.md` | 39 | 2 moderate |
| `docs/guides/MULTIMODEL_QUERIES.md` | 8 | 2 moderate |
| `docs/guides/QUANTIZATION.md` | 14 | 0 (verified) |
| `docs/guides/INSTALLATION.md` | 26 | 1 minor |
| `docs/guides/CONFIGURATION.md` | 18 | TBD |
| `docs/guides/TUTORIALS/MINI_RECOMMENDER.md` | 17 | TBD |
| `docs/guides/CLI_REPL.md` | 28 | TBD |

### Tier 3 -- Reference & Migration
| File | Code Blocks | Issues Found |
|------|-------------|--------------|
| `docs/MIGRATION_v1.4_to_v1.5.md` | 11 | 4 critical |
| `docs/VELESQL_SPEC.md` | 63 | 16 (FUSE BY not implemented) |
| `docs/getting-started.md` | 15 | 2 moderate |
| `docs/reference/api-reference.md` | 39 | 2 moderate |
| `docs/reference/NATIVE_HNSW.md` | 6 | 1 moderate |
| `docs/reference/VELESQL_SPEC.md` | 45 | TBD |

## All Confirmed Mismatches

### CRITICAL -- Nonexistent Methods/Classes

| # | File | Line(s) | Snippet | Issue | Fix |
|---|------|---------|---------|-------|-----|
| C1 | `README.md` | 66, 116 | `db.search("my_collection", vector=query_vec, top_k=10)` | `Database.search()` does NOT exist in Python SDK | `collection = db.get_collection("my_collection"); results = collection.search(vector=query_vec, top_k=10)` |
| C2 | `README.md` | 111-116 | `db.stream_insert("my_collection", points=[...])` | `stream_insert` is on Collection, not Database. Takes list of point dicts, no `points=` kwarg | `collection = db.get_collection("my_collection"); collection.stream_insert([...])` |
| C3 | `docs/guides/SEARCH_MODES.md` | 286-289 | `from velesdb import VelesDB; db = VelesDB("./data"); coll = db.collection("docs")` | Class is `velesdb.Database` not `VelesDB`. Method is `db.get_collection()` not `db.collection()` | `import velesdb; db = velesdb.Database("./data"); coll = db.get_collection("docs")` |
| C4 | `docs/MIGRATION_v1.4_to_v1.5.md` | 47-49 | `from velesdb import VelesDB; db = VelesDB("./data"); points = db.collection("my_collection").get_all()` | Three errors: wrong class name, wrong collection accessor, `get_all()` does not exist | `import velesdb; db = velesdb.Database("./data"); coll = db.get_collection("my_collection"); points = coll.get(coll.all_ids())` (or equivalent) |
| C5 | `docs/MIGRATION_v1.4_to_v1.5.md` | 64 | `db.collection("my_collection").upsert(points)` | `db.collection()` does not exist | `db.get_collection("my_collection").upsert(points)` |
| C6 | `docs/guides/SEARCH_MODES.md` | 82, 121, 140, 160, 579, 589, 709, 710 | `collection.search_with_quality(&query, 10, SearchQuality::Fast)` | `search_with_quality` is NOT a public method on `Collection`. It exists only on `HnswIndex` (internal) | Use `collection.search_with_ef(&query, 10, ef_search_value)?` or `collection.search(&query, 10)?` |
| C7 | `docs/reference/NATIVE_HNSW.md` | 91 | `search_with_quality(query, k, quality)` in API table | Same -- not a public Collection method | Document it as an HnswIndex method, or use the Collection equivalent |

### CRITICAL -- Fabricated VelesQL Syntax

| # | File | Line(s) | Snippet | Issue | Fix |
|---|------|---------|---------|-------|-----|
| C8 | `docs/guides/SEARCH_MODES.md` | 323, 329 + 2 more | `FUSE BY RRF(k=60)` / `FUSE BY RSF(...)` | `FUSE BY` is NOT in the pest grammar. Only `USING FUSION(...)` is implemented | Replace with `USING FUSION(strategy = 'rrf', k = 60)` |
| C9 | `docs/VELESQL_SPEC.md` | 16 occurrences | `FUSE BY` documented as stable v2.2 feature | Not implemented in grammar. Spec-vs-implementation gap | Either implement `FUSE BY` or mark as planned/remove |
| C10 | `README.md` | ~91 | `USING FUSION(strategy='rrf', k=60)` | This is correct syntax but shown differently in spec vs README (minor -- parses ok) | OK, no fix needed |

### MODERATE -- Wrong API Patterns

| # | File | Line(s) | Snippet | Issue | Fix |
|---|------|---------|---------|-------|-----|
| M1 | `README.md` | 428 | `collection.search([...], top_k=10)` | Python `search()` requires `vector=` keyword arg: `search(vector=None, *, sparse_vector=None, top_k=10)` | `collection.search(vector=[...], top_k=10)` |
| M2 | `crates/velesdb-python/README.md` | 155-157 | `collection.query("SELECT ...", ...)` missing space before `results` | Formatting issue (runon text) | Add newline |
| M3 | `crates/velesdb-python/README.md` | 113 | `multi_query_search` formatting -- `from velesdb import FusionStrategy` missing newline | Formatting issue | Add newline |
| M4 | `docs/guides/SEARCH_MODES.md` | 291-294 | `sparse_vector={"indices": [42, 156, 891], "values": [0.8, 0.3, 0.5]}` | Python SDK expects `{42: 0.8, 156: 0.3, 891: 0.5}` (dict[int,float]), not parallel arrays format | Fix to dict format |
| M5 | `docs/guides/SEARCH_MODES.md` | 351-353 | Same parallel-array sparse format in hybrid example | Same issue | Fix to dict format |
| M6 | `crates/velesdb-wasm/README.md` | 303 | `http://localhost:8080/v1/collections/docs/search` | REST API has NO `/v1/` prefix | `http://localhost:8080/collections/docs/search` |
| M7 | `crates/velesdb-wasm/README.md` | 311 | `http://localhost:8080/v1/query` | Same `/v1/` prefix issue | `http://localhost:8080/query` |
| M8 | `crates/velesdb-wasm/README.md` | 306 | `body: JSON.stringify({ vector: query, limit: 10 })` | REST uses `top_k`, not `limit` | `{ "vector": query, "top_k": 10 }` |
| M9 | `docs/guides/USE_CASES.md` | 202 | `import { VelesDB } from 'velesdb-client'` | Package is `@wiscale/velesdb-sdk`, not `velesdb-client` | Fix import path |
| M10 | `docs/guides/MULTIMODEL_QUERIES.md` | 26-28 | `import { VelesDB } from 'velesdb-client'; new VelesDB({baseUrl: ...})` | Same wrong package name | Fix import path |
| M11 | `docs/getting-started.md` | 21 | `langju/velesdb:latest` | Old Docker Hub image reference | `ghcr.io/cyberlife-coder/velesdb:latest` |
| M12 | `docs/getting-started.md` | 50 | `"version": "1.3.1"` | Outdated version | `"version": "1.5.0"` |
| M13 | `docs/reference/api-reference.md` | 714 | `point = collection.get(1)` | `get()` takes a list of IDs: `collection.get([1])` | Fix to list syntax |
| M14 | `docs/reference/api-reference.md` | 718 | `collection.search(query_vector, top_k=10)` | Missing `vector=` keyword | `collection.search(vector=query_vector, top_k=10)` |
| M15 | `docs/guides/INSTALLATION.md` | 151 | `collection.search(query_vector, top_k=10)` | Missing `vector=` keyword | `collection.search(vector=query_vector, top_k=10)` |

### MINOR -- Cosmetic / Accuracy

| # | File | Line(s) | Snippet | Issue | Fix |
|---|------|---------|---------|-------|-----|
| N1 | `crates/velesdb-core/README.md` | 376 | `use velesdb_core::{Filter, Condition}` | Verify these are actually publicly exported | Check `lib.rs` exports |
| N2 | `crates/velesdb-core/README.md` | 392 | `use velesdb_core::{recall_at_k, precision_at_k, mrr, ndcg_at_k}` | Verify these metric functions exist | Check `lib.rs` exports |
| N3 | `README.md` | Business scenarios | `NOW() - INTERVAL '7 days'`, `LINKED_TO*1..3` | PostgreSQL syntax not supported in VelesQL, no PSEUDOCODE markers | Add `-- PSEUDOCODE` comments or rewrite |

## Source of Truth Cross-Reference

### Python SDK (`crates/velesdb-python/src/`)

| Method | File | Correct Signature |
|--------|------|-------------------|
| `Database.__init__` | `lib.rs:204` | `Database(path: str)` |
| `Database.create_collection` | `lib.rs:234` | `create_collection(name, dimension, metric="cosine")` |
| `Database.get_collection` | `lib.rs:267` | `get_collection(name) -> Optional[Collection]` |
| `Database.train_pq` | `lib.rs:373` | `train_pq(collection_name, m=8, k=256, opq=False)` |
| `Collection.search` | `collection.rs:220` | `search(vector=None, *, sparse_vector=None, top_k=10)` |
| `Collection.search_with_ef` | `collection.rs:268` | `search_with_ef(vector, top_k=10, ef_search=128)` |
| `Collection.search_ids` | `collection.rs:286` | `search_ids(vector, top_k=10)` |
| `Collection.get` | `collection.rs:303` | `get(ids: List[int])` |
| `Collection.upsert` | `collection.rs:82` | `upsert(points: List[Dict])` |
| `Collection.stream_insert` | `collection.rs:821` | `stream_insert(points: List[Dict])` |
| `Collection.text_search` | `collection.rs:337` | `text_search(query, top_k=10, filter=None)` |
| `Collection.hybrid_search` | `collection.rs:356` | `hybrid_search(vector, query, top_k=10, vector_weight=0.5, filter=None)` |
| `Collection.query` | `collection.rs:478` | `query(query_str, params=None)` |

### Rust Core API (`crates/velesdb-core/src/`)

| Method | File | Correct Signature |
|--------|------|-------------------|
| `Database::open` | `database.rs:84` | `open(path: P) -> Result<Self>` |
| `Database::create_collection` | `database.rs:147` | `create_collection(name, dimension, metric) -> Result<()>` |
| `Database::create_collection_with_options` | `database.rs:168` | `create_collection_with_options(name, dim, metric, storage_mode) -> Result<()>` |
| `Collection::search` | `search/vector.rs:131` | `search(&self, query: &[f32], k: usize) -> Result<Vec<SearchResult>>` |
| `Collection::search_with_ef` | `search/vector.rs:183` | `search_with_ef(&self, query: &[f32], k: usize, ef_search: usize) -> Result<...>` |
| `Collection::search_ids` | `search/vector.rs:251` | `search_ids(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>>` |
| `Collection::text_search` | `search/text.rs:21` | `text_search(&self, query: &str, k: usize) -> Vec<SearchResult>` |
| `Collection::hybrid_search` | `search/text.rs:115` | `hybrid_search(&self, query_vec, text_query, k, vector_weight) -> Result<...>` |
| `Collection::upsert` | `core/crud.rs:24` | `upsert(&self, points: impl IntoIterator<Item = Point>) -> Result<()>` |
| `Collection::upsert_bulk` | `core/crud.rs:245` | `upsert_bulk(&self, points: &[Point]) -> Result<usize>` |

### WASM API (`crates/velesdb-wasm/src/lib.rs`)

| Method | Correct Signature |
|--------|-------------------|
| `VectorStore::new` | `new(dimension: usize, metric: &str)` |
| `VectorStore::insert` | `insert(id: u64, vector: &[f32])` |
| `VectorStore::search` | `search(query: &[f32], k: usize)` |
| `VectorStore::search_with_filter` | `search_with_filter(query, k, filter)` |
| `VectorStore::text_search` | `text_search(query, k, field)` |
| `VectorStore::hybrid_search` | `hybrid_search(vector, text_query, k, vector_weight, field)` |

### REST API Routes (no `/v1/` prefix)

Routes verified in `crates/velesdb-server/src/main.rs`:
- `POST /collections` -- create collection
- `GET /collections` -- list collections
- `GET /collections/{name}` -- collection info
- `DELETE /collections/{name}` -- delete collection
- `POST /collections/{name}/points` -- upsert points
- `POST /collections/{name}/search` -- vector/sparse/hybrid search
- `POST /collections/{name}/search/batch` -- batch search
- `POST /collections/{name}/search/multi` -- multi-query search
- `POST /collections/{name}/search/text` -- BM25 text search
- `POST /collections/{name}/search/hybrid` -- hybrid vector+text
- `POST /query` -- VelesQL query
- `POST /aggregate` -- aggregation query
- `POST /query/explain` -- explain plan

Search request body uses `top_k` (not `limit`).

## VelesQL Grammar vs Documentation Gap

The grammar (`grammar.pest`) supports:
- `USING FUSION(strategy = 'rrf', k = 60)` -- top-level fusion clause
- `vector NEAR_FUSED [$v1, $v2] USING FUSION 'rrf' (k=60)` -- multi-vector fusion
- `vector SPARSE_NEAR $sv` -- sparse vector search

The grammar does NOT support:
- `FUSE BY RRF(k=60)` -- documented in VELESQL_SPEC.md as stable, but not in grammar
- `NOW()` or `INTERVAL` -- used in README business scenarios
- `*1..3` variable-length graph traversal (e.g., `[:LINKED_TO*1..3]`)

## Don't Hand-Roll

| Problem | Don't Build | Use Instead | Why |
|---------|-------------|-------------|-----|
| Checking API signatures | Manual reading | `grep -n "pub fn\|fn " src/` against doc snippets | Exhaustive and reproducible |
| VelesQL syntax validation | Manual review | `cargo test velesql_parser_conformance` | Automated conformance suite |

## Common Pitfalls

### Pitfall 1: Positional vs Keyword Arguments in Python
**What goes wrong:** Python `search()` uses `(vector=None, *, sparse_vector=None, top_k=10)` -- the `*` means sparse_vector and top_k are keyword-only. Many docs show `search([...], top_k=10)` passing vector positionally, which works only because `vector` is the first param and CAN be positional. But the canonical form should use `vector=` for clarity.
**How to avoid:** Always use keyword form in docs: `search(vector=[...], top_k=10)`.

### Pitfall 2: Database vs Collection Method Scope
**What goes wrong:** Some docs call `db.search()` or `db.stream_insert()` which don't exist at the Database level. `search` is on Collection. `train_pq` IS on Database.
**How to avoid:** Check method location in source before documenting.

### Pitfall 3: Sparse Vector Format Varies by SDK
**What goes wrong:** REST accepts both `{"indices": [...], "values": [...]}` AND `{"42": 0.8}` dict format. Python SDK only accepts `{42: 0.8}` dict format (int keys, not string keys). SEARCH_MODES.md showed parallel-array format for Python which is wrong.
**How to avoid:** Use the correct format per SDK.

## Architecture Patterns

### Fix Strategy

Group fixes by file to minimize commits:

**Wave 1 (Critical): Root README + Migration Guide**
- `README.md`: Fix `db.search()`, `db.stream_insert()`, positional `search()`, business scenario PSEUDOCODE markers
- `docs/MIGRATION_v1.4_to_v1.5.md`: Fix `VelesDB` class, `db.collection()`, `get_all()`

**Wave 2 (Critical): SEARCH_MODES + VELESQL_SPEC**
- `docs/guides/SEARCH_MODES.md`: Fix `search_with_quality`, `VelesDB` import, `FUSE BY`, sparse format
- `docs/VELESQL_SPEC.md`: Fix or mark `FUSE BY` as unimplemented

**Wave 3 (Moderate): WASM + Guides**
- `crates/velesdb-wasm/README.md`: Fix `/v1/` prefix, `limit` -> `top_k`
- `docs/guides/USE_CASES.md`: Fix `velesdb-client` import
- `docs/guides/MULTIMODEL_QUERIES.md`: Fix `velesdb-client` import

**Wave 4 (Moderate): API Reference + Python README + remaining**
- `docs/reference/api-reference.md`: Fix `get(1)` -> `get([1])`, positional search
- `crates/velesdb-python/README.md`: Fix formatting issues
- `docs/guides/INSTALLATION.md`: Fix positional search
- `docs/getting-started.md`: Fix Docker image, version
- `docs/reference/NATIVE_HNSW.md`: Fix `search_with_quality` reference

### Verification Pattern

After each fix:
1. Grep for the OLD pattern to ensure zero remaining occurrences
2. Cross-reference the NEW pattern against the actual source signature
3. Run `cargo test` to ensure no doc-test breakage (for Rust snippets)

## Validation Architecture

### Test Framework
| Property | Value |
|----------|-------|
| Framework | Manual verification (text audit, not automated tests) |
| Quick run command | `grep -rn "db\.search\|VelesDB(\|db\.collection(\|FUSE BY\|/v1/" README.md crates/*/README.md docs/**/*.md` |
| Full suite command | `cargo test --workspace --features persistence,gpu,update-check --exclude velesdb-python -- --test-threads=1` |

### Phase Requirements -> Test Map
| Req ID | Behavior | Test Type | Automated Command | File Exists? |
|--------|----------|-----------|-------------------|-------------|
| DOC-01 | Python snippets match PyO3 signatures | manual grep | `grep -rn "db\.search\|VelesDB(" docs/ README.md` | N/A |
| DOC-02 | REST snippets match routes | manual grep | `grep -rn "/v1/" docs/ crates/*/README.md` | N/A |
| DOC-03 | VelesQL snippets parse cleanly | integration | `cargo test velesql_parser_conformance` | Yes |
| DOC-04 | No nonexistent methods in docs | manual grep | `grep -rn "search_with_quality\|db\.collection(\|get_all()" docs/` | N/A |

### Sampling Rate
- **Per task commit:** Quick grep for known-bad patterns
- **Per wave merge:** Full grep suite across all doc files
- **Phase gate:** Zero matches for all known-bad patterns

### Wave 0 Gaps
None -- this is a documentation-only phase, no test infrastructure needed.

## Sources

### Primary (HIGH confidence)
- `crates/velesdb-python/src/lib.rs` -- Python Database class methods
- `crates/velesdb-python/src/collection.rs` -- Python Collection methods and signatures
- `crates/velesdb-python/src/collection_helpers.rs` -- Sparse vector parsing
- `crates/velesdb-core/src/collection/search/vector.rs` -- Rust Collection search methods
- `crates/velesdb-core/src/collection/core/crud.rs` -- Rust Collection CRUD methods
- `crates/velesdb-core/src/database.rs` -- Rust Database public API
- `crates/velesdb-core/src/velesql/grammar.pest` -- VelesQL actual grammar
- `crates/velesdb-wasm/src/lib.rs` -- WASM VectorStore API
- `crates/velesdb-server/src/main.rs` -- REST API routes (no version prefix)
- `crates/velesdb-server/src/types.rs` -- REST request/response types

## Metadata

**Confidence breakdown:**
- Mismatch inventory: HIGH -- verified every finding against source code
- Fix recommendations: HIGH -- each fix maps directly to verified source signatures
- Completeness: MEDIUM -- Tier 3 files (CONFIGURATION.md, CLI_REPL.md, TUTORIALS) marked TBD

**Research date:** 2026-03-08
**Valid until:** 2026-04-08 (stable -- API unlikely to change in v1.5.x patch releases)
