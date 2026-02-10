---
phase: 4
plan: 2
completed: 2026-02-09
duration: ~30min
---

# Phase 4 Plan 02: SELECT Domain Scenarios — Summary

## One-liner

3 E2E tests validating NEAR + metadata filter queries (LIKE, BETWEEN, temporal/numeric comparisons, multi-column ORDER BY) for Medical Research, E-commerce, and Cybersecurity domain scenarios.

## What Was Built

Implemented SELECT-based domain scenario tests that exercise the full VelesQL query pipeline: parsing → vector search (NEAR) → metadata filtering → ORDER BY → LIMIT. Each test creates realistic domain data, executes the documented README query pattern, and asserts correctness of filtering, ordering, and result counts.

## Tasks Completed

| Task | Description | Status |
|------|-------------|--------|
| 1 | Scenario 1: Medical Research (NEAR + LIKE + string date comparison) | ✅ |
| 2 | Scenario 2: E-commerce (NEAR + BETWEEN + equality + multi ORDER BY) | ✅ |
| 3 | Scenario 3: Cybersecurity (NEAR + numeric + epoch temporal comparison) | ✅ |

## Key Files

**Modified:**
- `tests/readme_scenarios/select_domain.rs` — Replaced stub with 3 complete E2E tests (~400 lines)

**Helper added:**
- `embedding_to_json_param()` — Local utility to convert f32 vectors to JSON array params

## VelesQL Features Validated

| Feature | Scenario | Query Pattern |
|---------|----------|---------------|
| LIKE pattern matching | Medical | `content LIKE '%BRCA1%'` |
| String comparison | Medical | `publication_date > '2025-01-01'` |
| BETWEEN (numeric) | E-commerce | `price BETWEEN 20.0 AND 100.0` |
| Equality filter | E-commerce | `category = 'electronics'` |
| Numeric comparison | Cybersecurity | `threat_level > 0.8` |
| Integer comparison | Cybersecurity | `first_seen > 1735689600` |
| Multi-column ORDER BY | E-commerce, Cybersecurity | `similarity(...) DESC, field ASC` |
| NEAR + filter pipeline | All 3 | `vector NEAR $param AND ...` |

## Decisions Made

| Decision | Rationale |
|----------|-----------|
| Use `similarity(vector, $param)` in ORDER BY instead of bare `similarity()` | VelesQL grammar requires field + vector args in ORDER BY similarity expressions |
| Fixed epoch (1735689600) instead of `NOW() - INTERVAL` for Scenario 3 | Deterministic testing; temporal expression parsing validated separately |
| 128-dim vectors with 8-12 points per scenario | Realistic dimensions; small dataset ensures all points are HNSW candidates (4x overfetch covers all) |

## Deviations from Plan

- ORDER BY syntax adjusted from `similarity() DESC` to `similarity(vector, $param) DESC` — the VelesQL grammar requires explicit field and vector arguments
- No other deviations

## Verification Results

```
cargo test --test readme_scenarios -- --nocapture
  7 passed; 0 failed; 0 ignored (4 hero_query + 3 select_domain)

cargo clippy --test readme_scenarios -- -D warnings
  0 errors, 0 warnings
```

---
*Completed: 2026-02-09*
