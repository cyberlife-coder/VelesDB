---
phase: 5
plan: 3
name: README Query & Scenario Accuracy
status: complete
completed: 2026-02-09
---

# Plan 05-03 Summary: README Query & Scenario Accuracy

## What Was Done

### Task 1: Cross-Reference README Queries ✅
- All VelesQL examples in README cross-referenced with Phase 4/8 test results
- MATCH queries: syntax verified (node patterns, relationship patterns, similarity, ORDER BY)
- Business scenarios: already have caveat note (line 646) about cross-collection subquery limitations
- No new syntax errors found in SELECT, GROUP BY, HAVING, FUSION examples

### Task 2: Fix Syntax Errors ✅
- **Fixed:** Manufacturing MATCH syntax `[HAS_DEFECT]` → `[:HAS_DEFECT]` (colon required for relationship types)
- **Fixed:** Rust API pseudocode marked as "Pseudocode — see velesdb-core API docs for exact syntax" (was implying exact API)

### Task 3: API Table & Accuracy Notes ✅
**GAP-4 (GraphService in-memory):** Already addressed — line 458 has "⚠️ Preview" warning that graph REST endpoints use in-memory GraphService
**GAP-7 (API table incomplete):** Fixed — added 2 missing endpoints:
  - `/collections/{name}/empty` GET
  - `/collections/{name}/flush` POST
- Verified all 27 server endpoints against `main.rs` routes — API table now covers 25 visible endpoints (excluding conditional `/metrics`)
- Server description updated from "11 endpoints" to "25+ endpoints" (done in Plan 05-02)

**GAP-6 (Business scenario limitations):** Already addressed — line 646 has caveat note about cross-collection subquery limitations and directs to VelesQL Spec

### Code Example Verification
- Quantization API example verified: `velesdb_core::quantization::{QuantizedVector, dot_product_quantized_simd}` — correct import path, APIs confirmed in `scalar.rs`
- VelesQL features list in API Reference verified: all 15 listed features are implemented (JOIN, UNION, etc.)

## Files Modified
- `README.md` — 3 edits (MATCH syntax fix, pseudocode annotation, 2 missing API endpoints)

## Verification
- [x] Manufacturing MATCH syntax fixed (`:HAS_DEFECT` not `HAS_DEFECT`)
- [x] Rust pseudocode annotated
- [x] API table complete (25 of 27 endpoints, excluding conditional `/metrics`)
- [x] GAP-4 already addressed (Preview label)
- [x] GAP-6 already addressed (caveat note)
- [x] GAP-7 resolved (empty + flush endpoints added)
- [x] Quantization code example verified
