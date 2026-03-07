---
phase: 09-documentation
plan: 01
subsystem: documentation
tags: [readme, changelog, v1.5, badges, feature-cards]

# Dependency graph
requires:
  - phase: 08-sdk-parity
    provides: All v1.5 SDK features (sparse, PQ, streaming, hybrid) implemented
provides:
  - Updated README.md with v1.5 header, badges, What's New section, and roadmap
  - Complete CHANGELOG.md [Unreleased] section with all v1.5 entries
affects: [09-documentation, 10-release]

# Tech tracking
tech-stack:
  added: []
  patterns: [feature-card-with-code-examples, keep-a-changelog-subsections]

key-files:
  created: []
  modified:
    - README.md
    - CHANGELOG.md

key-decisions:
  - "What's New section placed between badge block and Problem We Solve section"
  - "v1.5 roadmap entry marked as released with 5 features, Distributed Mode moved to future"
  - "CHANGELOG preserves existing Expert Rust Review and SIMD consolidation entries"

patterns-established:
  - "Feature cards: short description + VelesQL/Python/REST code snippet per feature"

requirements-completed: [DOC-01, DOC-06]

# Metrics
duration: 4min
completed: 2026-03-07
---

# Phase 09 Plan 01: README + CHANGELOG v1.5 Summary

**README updated to v1.5.0 with What's New feature cards (PQ, Sparse, Hybrid, Streaming, Cache) and CHANGELOG completed with all v1.5 Added/Changed/Fixed/Security/Breaking entries**

## Performance

- **Duration:** 4 min
- **Started:** 2026-03-07T17:17:12Z
- **Completed:** 2026-03-07T17:21:11Z
- **Tasks:** 2
- **Files modified:** 2

## Accomplishments
- README header updated from v1.4.1 to v1.5.0 with new PQ and Sparse badges
- What's New in v1.5 section with 5 feature cards including VelesQL, Python, and REST code examples
- CHANGELOG [Unreleased] section completed with 40+ entries across Added, Changed, Fixed, Security, and Breaking Changes subsections
- Roadmap section updated: v1.5 marked as released, future timeline corrected

## Task Commits

Each task was committed atomically:

1. **Task 1: Update README.md to v1.5** - `3a2e1751` (docs)
2. **Task 2: Complete CHANGELOG.md v1.5 entries** - `b9c6d956` (docs)

## Files Created/Modified
- `README.md` - v1.5 header, badges, What's New section, updated quantization table, core features, roadmap
- `CHANGELOG.md` - Complete [Unreleased] v1.5 entries (PQ, Sparse, Hybrid, Streaming, Cache, SDK, Breaking Changes)

## Decisions Made
- What's New section placed immediately after badge block (before "The Problem We Solve") for maximum visibility
- Roadmap v1.5 entry changed from "Planned" to "Released" with actual features (PQ, Sparse, Streaming, Cache) instead of original placeholders (Distributed Mode moved to future)
- Existing [Unreleased] entries (Expert Rust Review, SIMD consolidation) preserved as-is, new v1.5 entries added after them
- CHANGELOG uses sub-headings within ### Added for each feature area (PQ, Sparse, Hybrid, Streaming, Cache, REST, SDK)

## Deviations from Plan

None - plan executed exactly as written.

## Issues Encountered
None

## User Setup Required
None - no external service configuration required.

## Next Phase Readiness
- README and CHANGELOG are v1.5-ready
- Remaining Phase 09 plans (rustdoc, OpenAPI, migration guide, benchmarks) can proceed
- No blockers

---
*Phase: 09-documentation*
*Completed: 2026-03-07*
