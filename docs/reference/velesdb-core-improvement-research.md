# VelesDB Core Improvement Research (Expert Brainstorm)

Last updated: 2026-03-20.

## Scope and method

This research focuses on **how to improve `velesdb-core`** while staying aligned with public product promises.

Inputs reviewed:

- Root `README.md` claims (latency, scale, quality, production readiness, local-first compliance positioning).
- `crates/velesdb-core/README.md` claims (core performance and capabilities).
- Existing internal production-readiness assessment (`docs/reference/velesdb-core-production-readiness.md`).
- Attempted direct fetch of `https://velesdb.com` from this environment (blocked with HTTP 403 via CONNECT tunnel), so website claims were validated indirectly through repository references to the “velesdb.com + README promise.”

## Promise baseline to protect (contract)

From the current docs, the core promise surface for `velesdb-core` can be summarized as:

1. **Ultra-low latency / high throughput** (sub-ms retrieval path, microsecond HNSW, nanosecond SIMD).  
2. **Unified query power** (vector + graph + structured filtering, VelesQL).  
3. **Production reliability** (thousands of tests, high coverage, security hygiene, deterministic behavior expectations).  
4. **Local-first deployment value** (small footprint, offline/air-gapped friendliness, compliance narrative).  

The most important strategic point: the remaining risk is not “missing fundamental features,” but **proof, consistency, and release-grade guarantees** around these promises.

## Multi-expert brainstorm

### Expert 1 — ANN/Search Systems Engineer

**Diagnosis**
- Search fundamentals are strong, but perceived performance can drift when benchmark conditions differ from real workload profiles (payload fetch, filtering, hybrid scoring, and planner branch choices).

**Top improvements**
- Build a **performance contract matrix** for every search profile (`Fast/Balanced/Accurate/Perfect`) with fixed datasets and hardware metadata.
- Add **continuous recall-vs-latency frontier tracking** in CI artifacts (not only point benchmarks).
- Introduce **tail-latency guards** (p95/p99 budgets) in addition to median latency.

**Expected impact**
- Protects the “100x faster / <1ms” narrative against regression and benchmark skepticism.

---

### Expert 2 — Query Planner & Database Engine Architect

**Diagnosis**
- VelesQL breadth is a strength; trust depends on deterministic planning and clear explainability when multiple strategies are possible.

**Top improvements**
- Freeze **deterministic planning invariants** with golden tests for `EXPLAIN` output on representative query classes.
- Add stricter **query contract tests** for placeholders, aliasing, mixed filters + vector clauses, and `MATCH` edge cases.
- Publish a compact **planner decision table** (input pattern → selected strategy → complexity intuition).

**Expected impact**
- Reduces "works but surprises me" outcomes; improves enterprise adoption confidence.

---

### Expert 3 — Reliability / SRE Engineer

**Diagnosis**
- Engine quality is already high, but public “production-ready” positioning requires explicit, repeatable release gates.

**Top improvements**
- Define a **production release checklist** enforced by CI (critical conformance suites, crash recovery scenarios, durability smoke, benchmark thresholds).
- Add **failure-mode drills**: corrupted segment simulation, interrupted flush, partial WAL replay (if applicable), and recovery-time SLO checks.
- Track **error-budget style regression policy** for performance and correctness issues across versions.

**Expected impact**
- Converts quality signals into operational guarantees.

---

### Expert 4 — Security & Compliance Engineer

**Diagnosis**
- Local-first compliance messaging is compelling, but needs stronger artifact-level proof for regulated adopters.

**Top improvements**
- Produce a **security evidence pack** per release: dependency audit snapshot, unsafe-code inventory delta, fuzzing status, and hardening checklist.
- Add **data handling posture docs** specific to `velesdb-core`: encryption-at-rest expectations, key ownership boundaries, telemetry defaults.
- Standardize **supply-chain attestations** (reproducible build notes/SBOM workflow where practical).

**Expected impact**
- Improves procurement success and trust in enterprise pipelines.

---

### Expert 5 — Developer Experience & Product Messaging

**Diagnosis**
- Promise risk often comes from docs drift and inconsistent examples rather than engine defects.

**Top improvements**
- Create a **single source-of-truth promise table** (claim, metric definition, benchmark command, last verified commit/date).
- Add CI that fails on **docs-vs-benchmark mismatch** for key numbers in README and crate README.
- Ship a **“real-world profile” benchmark preset** that includes payload retrieval, filtering, and hybrid scoring by default.

**Expected impact**
- Makes claims reproducible and reduces trust friction for users evaluating the project.

## Consolidated priority roadmap

### P0 (next release cycle)

1. **Promise contract registry + CI checks**  
   - Machine-readable file for key public claims and validation commands.
2. **Mandatory production gate suite**  
   - Critical VelesQL conformance + crash recovery + selected perf thresholds.
3. **Deterministic planner golden tests**  
   - Freeze planner behavior for high-value query families.

### P1 (1–2 release cycles)

4. **Tail-latency and frontier reporting**  
   - p50/p95/p99 + recall frontier chart artifacts per release.
5. **Security evidence pack**  
   - Audit/fuzz/supply-chain snapshot attached to releases.
6. **Docs and benchmark de-drift automation**  
   - README/core README numbers validated against benchmark output.

### P2 (later hardening)

7. **Failure-injection harness expansion**  
   - Automated scenarios for corruption/interrupted persistence paths.
8. **Workload profile catalog**  
   - Standardized benchmark suites by use case (RAG, recommendations, graph-heavy).

## Suggested KPI dashboard for `velesdb-core`

- **Correctness**: VelesQL conformance pass rate, deterministic planner snapshot stability.
- **Performance**: p50/p95/p99 latency by profile, recall@k, throughput by workload profile.
- **Reliability**: recovery success rate, recovery time, durability scenario pass rate.
- **Security**: advisory count, fuzz coverage trend, unsafe-code delta.
- **Promise integrity**: % of published claims with automated proof in CI.

## Practical conclusion

`velesdb-core` appears technically strong and feature-rich. The highest-ROI improvements now are **contract enforcement, deterministic behavior guarantees, release gating, and evidence automation**. These changes would make the public value proposition significantly more defensible while preserving the project’s current performance and local-first differentiation.

## Priority completion status (implemented)

All roadmap priorities are now implemented as concrete repository artifacts and checks:

- **P0.1 Promise contract registry + CI checks** ✅  
  - `docs/reference/promise-contract.json`  
  - `scripts/check-promise-contract.py`  
  - `.github/workflows/promise-contract.yml`
- **P0.2 Mandatory production gate suite** ✅  
  - `scripts/run-production-gates.sh`  
  - `.github/workflows/production-gates.yml`
- **P0.3 Deterministic planner golden tests** ✅  
  - `crates/velesdb-core/tests/velesql_planner_golden.rs`
- **P1.4 Tail latency/frontier reporting** ✅  
  - `scripts/generate-performance-proofpack.py`  
  - `benchmarks/proofpack-sample.json`  
  - generated `docs/reference/performance-proofpack.md`
- **P1.5 Security evidence pack** ✅  
  - `scripts/generate-security-evidence-pack.sh`  
  - generated `docs/reference/security-evidence-pack.md`
- **P1.6 Docs-benchmark de-drift automation** ✅  
  - `scripts/check-promise-contract.py` validates key README/core README claims through registry entries.
- **P2.7 Failure-injection harness expansion** ✅  
  - `scripts/run-failure-injection-harness.sh`
- **P2.8 Workload profile catalog** ✅  
  - `docs/reference/workload-profile-catalog.md`
