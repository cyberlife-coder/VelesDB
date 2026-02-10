# README vs Reality — GAP Analysis

**Audit date:** 2026-02-09
**Auditor:** Cascade (systematic codebase verification)
**Branch:** main (post v2-core-trust)

---

## Summary

| Severity | Count | Description |
|----------|-------|-------------|
| 🔴 **FALSE** | 0 | ~~Feature claimed as working but NOT executed~~ — All resolved in Phase 8 |
| 🟡 **MISLEADING** | 5 | Partially true but missing critical context |
| 🟢 **STALE** | 2 | Outdated numbers or references |

---

## 🔴 FALSE Claims — ALL RESOLVED ✅

### GAP-1: JOIN Execution — ✅ RESOLVED (Phase 8, Plans 08-01/08-02)

`Database::execute_query()` now handles JOIN via ColumnStore bridge. INNER and LEFT JOIN fully executed.
- Commit: Plans 08-01 (`database_query.rs`), 08-02 (JOIN wiring)
- Tests: 7 JOIN integration tests in `database_query_tests.rs`, 6 E2E tests in `tests/e2e_join.rs`
- README updated: `✅ JOIN (INNER, LEFT) across collections`

---

### GAP-2: UNION / INTERSECT / EXCEPT Set Operations — ✅ RESOLVED (Phase 8, Plans 08-01/08-03)

`Database::execute_query()` now handles compound queries (UNION, UNION ALL, INTERSECT, EXCEPT).
- Commit: Plans 08-01 (`compound.rs`), 08-03 (integration tests)
- Tests: 9 unit tests in `compound_tests.rs`, 7 integration tests in `database_query_tests.rs`, 5 E2E tests in `tests/e2e_compound.rs`
- README updated: `✅ UNION / INTERSECT / EXCEPT set operations`

---

### GAP-3: `/query/explain` Endpoint Not Routed — ✅ RESOLVED (Phase 8, Plan 08-04)

`/query/explain` route added to `main.rs` line 122. Server handler delegates to `Database::execute_query()` for cross-collection support.
- Commit: Plan 08-04 (`main.rs`, `handlers/query.rs`)
- README updated: `/query/explain` added to API table

---

## 🟡 MISLEADING Claims (Partially true, missing context)

### GAP-4: GraphService Server Persistence

**README claims (line 452-459):** Graph endpoints listed as standard features.

**Reality:**
- `main.rs` line 66-72: GraphService is explicitly **IN-MEMORY ONLY**
- Server logs warning: "Graph data is in-memory only and will NOT persist across restarts"
- The core `EdgeStore` within `Collection` IS persistent (via mmap)
- But the server's `/graph/*` endpoints use a separate `GraphService` that loses all data on restart

**Impact:** Users adding edges via REST API will lose all graph data on server restart.

**Recommendation:** Add clear "⚠️ Preview — in-memory only" label to graph REST endpoints in README.

---

### GAP-5: Performance Number Conflicts

**README line 11/27/120:** "18.7ns" Dot Product 768D
**README line 1048:** "46 ns" Dot Product 768D

**Reality:**
- 18.7ns is from i9-14900KF with AVX2 4-accumulator optimization (specific hardware)
- 46ns is from a different/older benchmark run or different hardware
- Both appear in README without hardware context
- Additional metrics table (line 927-933) shows yet another set of numbers

**Impact:** Contradictory numbers damage credibility.

**Fix:** Use ONE consistent set of numbers with clear hardware context, or show "best case" vs "typical" clearly.

---

### GAP-6: Business Scenario Queries Mixing Working + Non-Working Patterns

**README Scenarios 1-4 (lines 628-727)** show queries like:
```sql
MATCH (product:Product)-[:SUPPLIED_BY]->(supplier:Supplier)
WHERE 
  similarity(product.image_embedding, $uploaded_photo) > 0.7
  AND supplier.trust_score > 4.5
  AND (SELECT price FROM inventory WHERE sku = product.sku) < 500
ORDER BY similarity() DESC
LIMIT 12
```

**Reality:**
- ✅ MATCH pattern matching — works
- ✅ similarity() in WHERE — works
- ✅ Scalar subqueries in MATCH WHERE — works (VP-002)
- ✅ ORDER BY similarity() — works
- ❌ The subquery references `product.sku` as correlated outer reference — **correlated cross-collection subqueries are limited**
- ❌ `NOW() - INTERVAL '24 hours'` — temporal arithmetic parses but execution converts to epoch seconds, not SQL-style interval math
- ❌ Nested SELECT inside MATCH referencing separate tables (like `FROM inventory`, `FROM transactions`) — requires cross-collection access that isn't implemented in MATCH executor

**Impact:** Business scenarios look impressive but some specific patterns would fail at execution.

**Recommendation:** Either verify each business scenario query works end-to-end with a test, or mark them as "Vision" / "Planned" examples.

---

### GAP-7: README API Table Is Incomplete

**README API section (lines 425-501)** lists endpoints but misses several that actually exist.

**Missing from README but routed in server:**
| Endpoint | Method | Description |
|----------|--------|-------------|
| `/collections/{name}/empty` | GET | Check if collection is empty |
| `/collections/{name}/flush` | POST | Flush collection to disk |
| `/collections/{name}/search/text` | POST | BM25 text search |
| `/collections/{name}/search/hybrid` | POST | Combined vector + text search |
| `/collections/{name}/match` | POST | Execute MATCH graph query |

**Previously missing route (now fixed):**
| Endpoint | Status |
|----------|--------|
| `/query/explain` | ✅ Routed (Phase 8, Plan 08-04) |

---

### GAP-8: "100% Ecosystem Complete" Claim

**README line 15:** "100% Ecosystem Complete"

**Reality:**
- All 8 crates exist and compile ✅
- But install commands may not work if packages aren't published to registries:
  - `cargo add velesdb-core` — may not be on crates.io
  - `pip install velesdb` — may not be on PyPI
  - `npm i @wiscale/velesdb` — may not be on npm
- `velesdb-mobile` has 5 source files — functional but minimal

**Recommendation:** Verify registry publication status or add "(from source)" annotations.

---

## 🟢 STALE Numbers

### GAP-9: Test Count

**README claims:** "3,100+" tests in multiple places (lines 11, 28, 94, 142)
**STATE.md claims:** 3,222 tests
**Quality Gates (line 142):** "3,000 passing"

**Fix:** Run `cargo test --workspace` and update ALL references consistently.

### GAP-10: Crate Count

**README line 132:** "8 production crates"
**Actual:** 8 crates in `crates/` directory — this is correct ✅

---

## Action Items

| Priority | GAP | Action |
|----------|-----|--------|
| ✅ | GAP-3 | `/query/explain` routed (Phase 8, Plan 08-04) |
| ✅ | GAP-1 | JOIN fully executed (INNER, LEFT) — Phase 8, Plans 08-01/08-02 |
| ✅ | GAP-2 | Set operations fully executed — Phase 8, Plans 08-01/08-03 |
| P1 | GAP-4 | Add "⚠️ Preview" label to graph REST endpoints |
| P1 | GAP-5 | Reconcile performance numbers with hardware context |
| P1 | GAP-6 | Add E2E tests for business scenario queries OR mark as "Vision" |
| P1 | GAP-7 | Add missing endpoints to README API table |
| P2 | GAP-8 | Verify registry publication or annotate install commands |
| P2 | GAP-9 | Update test counts to current numbers |
