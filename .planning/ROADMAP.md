# Roadmap: VelesDB

## Milestones

- ✅ **v1.5 Open-Core Parity** — Phases 1-16 (shipped 2026-03-08)
- 🔄 **v1.6 Hybrid Query Credibility** — Phase 17 (in progress)

## Phases

<details>
<summary>✅ v1.5 Open-Core Parity (Phases 1-16) — SHIPPED 2026-03-08</summary>

- [x] Phase 1: Quality Baseline & Security (4/4 plans) — completed 2026-03-06
- [x] Phase 2: PQ Core Engine (4/4 plans) — completed 2026-03-06
- [x] Phase 3: PQ Integration (3/3 plans) — completed 2026-03-06
- [x] Phase 4: Sparse Vector Engine (3/3 plans) — completed 2026-03-06
- [x] Phase 5: Sparse Integration (4/4 plans) — completed 2026-03-06
- [x] Phase 6: Query Plan Cache (2/2 plans) — completed 2026-03-07
- [x] Phase 7: Streaming Inserts (3/3 plans) — completed 2026-03-07
- [x] Phase 8: SDK Parity (4/4 plans) — completed 2026-03-07
- [x] Phase 9: Documentation (4/4 plans) — completed 2026-03-07
- [x] Phase 10: Release Readiness (2/2 plans) — completed 2026-03-07
- [x] Phase 11: PQ Recall Benchmark Hardening (2/2 plans) — completed 2026-03-08
- [x] Phase 12: Traceability & Cosmetic Cleanup (1/1 plan) — completed 2026-03-08
- [x] Phase 13: Recall Benchmark Multi-Distribution (1/1 plan) — completed 2026-03-08
- [x] Phase 14: README Documentation Audit (2/2 plans) — completed 2026-03-08
- [x] Phase 15: LangChain & LlamaIndex v1.5 Parity (2/2 plans) — completed 2026-03-08
- [x] Phase 16: Traceability & EXPLAIN Cosmetic Fixes (1/1 plan) — completed 2026-03-08

Full details: `.planning/milestones/v1.5-ROADMAP.md`

</details>

### Phase 17: Hybrid Query Test & Demo Coverage

**Goal:** Prove VelesDB's core value proposition with executable tests and real demos. Every claim — vector+graph+sparse in one query — must be backed by an integration test that executes the query end-to-end, asserts ranked results, and validates multi-signal fusion. Demos must run without scaffolding.

**Milestone:** v1.6 Hybrid Query Credibility

**Requirements:** HYB-01, HYB-02, HYB-03, HYB-04, HYB-05

| ID | Requirement |
|----|-------------|
| HYB-01 | Integration test executes a single VelesQL MATCH+similarity+scalar-filter query on a controlled corpus and asserts result identity and ranking order |
| HYB-02 | Integration test validates BM25+cosine hybrid fusion: corpus where signals diverge, assert fusion outranks each signal alone |
| HYB-03 | Integration test uses real GraphCollection edges and validates MATCH traversal combined with similarity in one executed query |
| HYB-04 | Python examples use real VelesDB API calls (not pseudocode/print); non-runnable examples clearly labeled as pseudocode |
| HYB-05 | `ecommerce_recommendation` example demonstrates Vector+Graph fusion via VelesQL or engine-level call, not manual HashMap merge |

**Plans:** 2/2 plans complete

Plans:
- [ ] 17-01-PLAN.md — Integration tests: HYB-01 (NEAR + scalar filter + ranking identity), HYB-02 (fusion ranking differs from pure vector), HYB-03 (graph MATCH traversal with real edges)
- [ ] 17-02-PLAN.md — Example fixes: HYB-05 (ecommerce QUERY 4 → hybrid_search()), HYB-04 (Python PSEUDOCODE headers)

## Progress

| Phase | Milestone | Plans Complete | Status | Completed |
|-------|-----------|----------------|--------|-----------|
| 1. Quality Baseline & Security | v1.5 | 4/4 | Complete | 2026-03-06 |
| 2. PQ Core Engine | v1.5 | 4/4 | Complete | 2026-03-06 |
| 3. PQ Integration | v1.5 | 3/3 | Complete | 2026-03-06 |
| 4. Sparse Vector Engine | v1.5 | 3/3 | Complete | 2026-03-06 |
| 5. Sparse Integration | v1.5 | 4/4 | Complete | 2026-03-06 |
| 6. Query Plan Cache | v1.5 | 2/2 | Complete | 2026-03-07 |
| 7. Streaming Inserts | v1.5 | 3/3 | Complete | 2026-03-07 |
| 8. SDK Parity | v1.5 | 4/4 | Complete | 2026-03-07 |
| 9. Documentation | v1.5 | 4/4 | Complete | 2026-03-07 |
| 10. Release Readiness | v1.5 | 2/2 | Complete | 2026-03-07 |
| 11. PQ Recall Benchmark Hardening | v1.5 | 2/2 | Complete | 2026-03-08 |
| 12. Traceability & Cosmetic Cleanup | v1.5 | 1/1 | Complete | 2026-03-08 |
| 13. Recall Benchmark Multi-Distribution | v1.5 | 1/1 | Complete | 2026-03-08 |
| 14. README Documentation Audit | v1.5 | 2/2 | Complete | 2026-03-08 |
| 15. LangChain & LlamaIndex v1.5 Parity | v1.5 | 2/2 | Complete | 2026-03-08 |
| 16. Traceability & EXPLAIN Cosmetic Fixes | v1.5 | 1/1 | Complete | 2026-03-08 |
| 17. Hybrid Query Test & Demo Coverage | 2/2 | Complete   | 2026-03-08 | — |
