---
phase: 4
plan: 1
completed: 2026-02-08
duration: ~2h
---

# Phase 4 Plan 01: Test Infrastructure & Hero Query — Summary

## One-liner

E2E test module `tests/readme_scenarios/` with shared helpers and 4 passing hero query tests validating graph traversal, property filtering, similarity scoring, and cross-node projection.

## What Was Built

Created the complete integration test infrastructure for README scenario validation. The module provides reusable helpers for DB setup, deterministic vector generation, labeled node insertion, edge creation, and programmatic `MatchClause` construction with OpenCypher-style inline property filtering.

The hero query test validates the defining VelesDB query from the README: `MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)` with category filtering, similarity scoring on end nodes, ORDER BY similarity() DESC, and cross-node property projection.

## Tasks Completed

| Task | Description | Files |
|------|-------------|-------|
| 1 | Module structure + shared helpers | `main.rs`, `helpers.rs` |
| 2 | Hero query tests (4 tests) | `hero_query.rs` |
| 3 | Stub files for Wave 2 plans | 5 stub files |

## Key Files

**Created:**
- `tests/readme_scenarios/main.rs` — Module root with 7 mod declarations
- `tests/readme_scenarios/helpers.rs` — Shared utilities (setup_test_db, generate_embedding, setup_labeled_collection, insert_labeled_nodes, add_edges, build_single_hop_match, build_match_clause)
- `tests/readme_scenarios/hero_query.rs` — 4 hero query tests
- `tests/readme_scenarios/select_domain.rs` — Stub for Plan 04-02
- `tests/readme_scenarios/metrics_and_fusion.rs` — Stub for Plan 04-03
- `tests/readme_scenarios/match_simple.rs` — Stub for Plan 04-04
- `tests/readme_scenarios/match_complex.rs` — Stub for Plan 04-05
- `tests/readme_scenarios/cross_store.rs` — Stub for Plan 04-06

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Use `NodePattern.properties` for start-node filtering instead of WHERE | Single-hop WHERE evaluates on target (end) node; properties filter at start-node discovery |
| Split similarity test from graph traversal test | `execute_match_with_similarity` scores on end node (Person), requiring separate vector design |
| `generate_embedding` uses sin-based deterministic formula | Reproducible, normalized vectors with controlled similarity via seed proximity |

## Deviations from Plan

- Plan specified `parse_and_execute_query` helper — deferred (no VelesQL parser usage needed yet; Wave 2 plans may add it)
- WHERE clause replaced by `NodePattern.properties` for category filtering — single-hop WHERE evaluates on target node, not start node

## Verification Results

```
cargo test --test readme_scenarios -- --nocapture
  4 passed; 0 failed; 0 ignored

cargo clippy --test readme_scenarios -- -D warnings
  0 errors, 0 warnings (only pre-existing lib warning about dual clippy.toml)
```

---
*Completed: 2026-02-08*
