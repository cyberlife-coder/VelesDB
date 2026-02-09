---
phase: 5
plan: 1
name: VelesQL Spec & Feature Matrix Truth
status: complete
completed: 2026-02-09
---

# Plan 05-01 Summary: VelesQL Spec & Feature Matrix Truth

## What Was Done

### Task 1: Feature Support Status Table ✅
- Verified Phase 8 entries (JOIN, UNION/INTERSECT/EXCEPT, EXPLAIN) all present and correct
- **Fixed:** RIGHT/FULL JOIN status from "🧪 Parser only (falls back to INNER)" → "⚠️ Parsed, returns `UnsupportedFeature` error" (matches actual code in `join.rs:162-170`)
- **Added:** `JOIN ... USING (col)` as "🧪 Parser only, returns `UnsupportedFeature` error"
- **Added:** `Database::execute_query()` cross-collection execution as "✅ Stable | 2.2"
- Verified Table aliases still shows "✅ Stable | 2.0" — correct

### Task 2: Subquery and NEAR_FUSED Documentation ✅
- **Already present:** Scalar Subqueries section (lines 530-575) with syntax, examples, limitations
- **Already present:** NEAR_FUSED section (lines 576-625) with strategies and examples
- No changes needed — these were added in a prior session

### Task 3: EBNF Grammar ✅
- **Already complete:** Full EBNF grammar (lines 826-963) includes:
  - MATCH clause (match_stmt, graph_pattern, node_pattern, relationship_pattern)
  - Temporal expressions (temporal_expr, now_func, interval_expr, temporal_arith)
  - Subquery (subquery_expr as value type)
  - NEAR_FUSED (fused_search)
  - ILIKE (alongside LIKE in like_cond)
  - JOIN clause (join_clause, join_type)
  - Compound queries (set_operator in top-level query)
- **Fixed:** JOIN EBNF comment updated to match reality ("returns UnsupportedFeature error" not "parsed only")

### Task 4: FEATURE_TRUTH.md Consistency ✅
- Verified FEATURE_TRUTH.md already had correct RIGHT/FULL JOIN status (⚠️ Caveat)
- Verified JOIN USING shows 🟡 Parse-only — consistent with VELESQL_SPEC.md
- **Added:** `Database::execute_query()` cross-collection as explicit feature row
- No contradictions between FEATURE_TRUTH.md and VELESQL_SPEC.md

## Files Modified
- `docs/VELESQL_SPEC.md` — 5 edits (RIGHT/FULL JOIN fix, JOIN USING added, Database::execute_query() added, JOIN section text, EBNF comment)
- `.planning/phases/v4-05-readme-documentation-truth/FEATURE_TRUTH.md` — 1 edit (Database::execute_query() row added)

## Verification
All success criteria met:
- [x] Feature table verified with Phase 8 entries + new features
- [x] RIGHT/FULL JOIN → UnsupportedFeature documented
- [x] Database::execute_query() documented
- [x] Subquery documentation section present
- [x] NEAR_FUSED documentation section present
- [x] EBNF grammar complete (MATCH, temporal, subquery, NEAR_FUSED, ILIKE, JOIN, compound)
- [x] FEATURE_TRUTH.md consistent with VELESQL_SPEC.md
- [x] No markdown formatting errors
