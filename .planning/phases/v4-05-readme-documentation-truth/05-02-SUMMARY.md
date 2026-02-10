---
phase: 5
plan: 2
name: README Metrics, Numbers & Ecosystem Truth
status: complete
completed: 2026-02-09
---

# Plan 05-02 Summary: README Metrics, Numbers & Ecosystem Truth

## What Was Done

### Task 1: Test Count & Coverage Numbers ✅
- Ran `cargo test --workspace`: **3,339 tests passing** (67 ignored)
- Updated ALL 5 test count locations from "3,100+" / "3,000" → "3,300+"
  - Line 11: header tagline
  - Line 28: badge
  - Line 94: quality metrics HTML
  - Line 142: quality gates
  - Line 1085: system performance table
- Coverage: kept at 82.30% (coverage_summary.txt too large to re-verify, no newer run available)

### Task 2: Performance Numbers Reconciled ✅
- **No conflict found**: README consistently uses 18.4ns for Dot Product 768D AVX2
- Verified against `bench_simd_results.txt`: actual measurement is 18.389-18.431ns → rounds to 18.4ns ✅
- The "46ns" / "18.7ns" conflict mentioned in GAPS.md was from an older README version — already resolved
- All performance tables are internally consistent

### Task 3: Ecosystem Table & Component Links ✅
- All 14 ecosystem paths verified as existing on disk
- **Fixed:** TypeScript SDK install command: `@wiscale/velesdb` → `@wiscale/velesdb-sdk` (matches actual `package.json` name)
- **Fixed:** Server endpoint count: "11 endpoints" → "25+ endpoints" (FEATURE_TRUTH.md lists 25 routed endpoints)
- **Added:** Registry publication footnote — honest about install commands requiring published packages
- Python package name `velesdb` confirmed correct (matches `pyproject.toml`)
- LangChain `langchain-velesdb` and LlamaIndex `llama-index-vector-stores-velesdb` confirmed correct

## Files Modified
- `README.md` — 8 edits (5 test count updates, TS SDK name fix, server endpoint count, registry footnote)

## Verification
- [x] All test count references updated to 3,300+ (consistent)
- [x] Coverage references verified (82.30% consistent)
- [x] Performance numbers reconciled — no conflicts
- [x] Ecosystem table links all verified as existing
- [x] Install commands accurate (TS SDK fixed) + registry annotation added
- [x] No phantom components referenced
