# Phase 7: Cross-Store Execution & EXPLAIN Completeness

**Milestone:** v4-verify-promise
**Requirements:** VP-010 (remaining), VP-013 (new — EXPLAIN completeness)
**Estimate:** 8-12h
**Wave:** 1 (all plans sequential)
**Depends on:** Phase 6 ✅

---

## Goal

Complete the cross-store query execution engine and EXPLAIN support that were identified as gaps during Phase 6. Currently:

- **GraphFirst** exists via `execute_match_with_similarity()` — but VectorFirst and Parallel strategies are **not implemented**
- `QueryPlanner::choose_hybrid_strategy()` exists in `planner.rs` but is **never called** from `execute_query()`
- EXPLAIN system lacks plan nodes for **NEAR_FUSED** and **cross-store (V+G)** queries

## Current State (post-Phase 6)

| Component | Status | Location |
|-----------|--------|----------|
| `QueryPlanner` with VectorFirst/GraphFirst/Parallel | ✅ Exists, never wired | `velesql/planner.rs` |
| `choose_hybrid_strategy()` | ✅ Exists, never called | `velesql/planner.rs:248` |
| `HybridExecutionPlan` | ✅ Exists, unused | `velesql/planner.rs:329` |
| `execute_match_with_similarity()` | ✅ GraphFirst only | `match_exec/similarity.rs:109` |
| VectorFirst execution | ❌ Missing | needs `cross_store_exec.rs` |
| Parallel execution + fuse | ❌ Missing | needs `cross_store_exec.rs` |
| EXPLAIN for NEAR_FUSED | ❌ Missing | `velesql/explain.rs` — no FusedSearch PlanNode |
| EXPLAIN for cross-store | ❌ Missing | `velesql/explain.rs` — no CrossStoreSearch PlanNode |

## Key Integration Points

1. **`execute_query()` in `mod.rs`**: detect `query.match_clause.is_some()` + vector search → route to cross-store
2. **`dispatch.rs`**: add `dispatch_cross_store()` calling VectorFirst/Parallel strategies
3. **`cross_store_exec.rs`** (new): VectorFirst + Parallel execution
4. **`explain.rs`**: add `FusedSearch` and `CrossStoreSearch` PlanNode variants
5. **`formatter.rs`**: render new plan nodes

## Plans

| Plan | Title | Scope | Estimate |
|------|-------|-------|----------|
| 07-01 | Cross-Store VectorFirst & Parallel Execution | TDD + `cross_store_exec.rs` + wire QueryPlanner | 4-6h |
| 07-02 | EXPLAIN Nodes for NEAR_FUSED & Cross-Store | TDD + PlanNode variants + rendering | 2-3h |
| 07-03 | Integration & Quality Gates | E2E tests + all quality gates | 2-3h |

## Success Criteria

- [ ] `QueryPlanner::choose_hybrid_strategy()` called from `execute_query()` for combined V+G queries
- [ ] VectorFirst: vector NEAR search → graph MATCH validate → filter → results
- [ ] Parallel: concurrent V+G → fuse with RRF → filter → results
- [ ] EXPLAIN shows `FusedSearch` node for NEAR_FUSED queries
- [ ] EXPLAIN shows `CrossStoreSearch` node with strategy/over-fetch/cost
- [ ] 15+ new tests covering cross-store execution + EXPLAIN
- [ ] `mod.rs` stays under 300 lines
- [ ] All quality gates pass (fmt, clippy, deny, test, release build)
