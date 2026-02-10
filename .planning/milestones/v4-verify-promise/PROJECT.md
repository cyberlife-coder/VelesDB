# VelesDB Core — Milestone v4: Verify Promise

## What This Is

A comprehensive audit-and-fix milestone that validates whether VelesDB **actually delivers** what its README, GitHub page, and website promise. Every business scenario, every code example, every feature claim is tested end-to-end. Where gaps exist, they are either **implemented** or the documentation is **corrected** to match reality.

> **Principle: "If we promise it, it must work. If it doesn't work, we either fix it or stop promising it."**

## Core Value

After this milestone, every query example in the README executes correctly and returns the expected results. Users who copy-paste examples get working code, not silent failures or incorrect results.

## Context

### The Promise (README / GitHub / Website)

VelesDB positions itself as a **unified engine replacing 3 databases** (Vector + Graph + Column) with a single query language (VelesQL). The README contains:

- **4 business scenarios** (E-commerce, Fraud Detection, Healthcare, AI Agent Memory) using MATCH + similarity() + subqueries
- **3 technical deep-dives** (Scenario 0/0b/0c) showing cross-store queries
- **3 domain scenarios** (Medical Research, E-commerce Recommendations, Cybersecurity)
- **Website screenshots** promising: Agentic Memory, GraphRAG, AI Desktop Apps, Browser Vector Search, Mobile AI, Robotics, On-Premises, Multi-Agent Collaboration

### The Reality (Code Audit — 2026-02-08)

| Feature | Parser | Executor (SELECT) | Executor (MATCH) | Status |
|---------|--------|-------------------|-------------------|--------|
| Basic comparisons (=, <>, <, >) | ✅ | ✅ | ✅ | **WORKS** |
| LIKE / ILIKE | ✅ | ✅ | ❌ `_ => Ok(true)` | **BROKEN in MATCH** |
| BETWEEN | ✅ | ✅ | ❌ `_ => Ok(true)` | **BROKEN in MATCH** |
| IN (value list) | ✅ | ✅ | ❌ `_ => Ok(true)` | **BROKEN in MATCH** |
| similarity() threshold | ✅ | ✅ | ✅ | **WORKS** |
| NOW() - INTERVAL | ✅ parse | ✅ epoch conversion | ❌ not wired | **BROKEN in MATCH** |
| Subqueries (SELECT...) | ✅ parse | ❌ → Value::Null | ❌ not evaluated | **FAKE** |
| Variable-length paths `*1..3` | ✅ parse | N/A | ⚠️ depth-only | **INCOMPLETE** |
| Multi-hop MATCH chains | ✅ parse | N/A | ⚠️ first pattern only | **INCOMPLETE** |
| RETURN with aggregation | ✅ parse | N/A | ❌ not implemented | **FAKE** |
| Cross-store MATCH+Column | ❌ | N/A | ❌ | **NOT IMPLEMENTED** |
| ORDER BY in MATCH | ✅ | N/A | ⚠️ similarity() only | **INCOMPLETE** |
| NEAR_FUSED fusion | ✅ | ✅ | N/A | **WORKS** |
| 5 distance metrics | ✅ | ✅ | ✅ | **WORKS** |
| Graph BFS/DFS traversal | N/A | N/A | ✅ | **WORKS** |

### Critical Gap: The Hero Query

The defining query in the README (line 163-170):
```sql
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
  AND doc.category = 'research'
RETURN author.name, author.email, doc.title
ORDER BY similarity() DESC
LIMIT 5;
```

**Reality:** This query **partially works** — MATCH traversal + similarity threshold works, but:
- Property projection from `author.name` across relationship bindings: **works** (EPIC-058)
- `doc.category = 'research'` comparison in MATCH WHERE: **works**
- ORDER BY similarity() DESC: **works**

**Verdict: Hero query WORKS for basic cases.** ✅

### Critical Gap: Business Scenario Queries

ALL 4 business scenarios use this pattern:
```sql
MATCH (a)-[:REL]->(b)
WHERE similarity(...) > X
  AND b.property = value
  AND (SELECT col FROM table WHERE ...) > threshold  -- ❌ SUBQUERY = NULL
```

The `(SELECT ... FROM ... WHERE ...)` subqueries are **parsed but evaluate to NULL**, making the entire condition silently pass. **Users get incorrect results without any error.**

## Requirements

### Active

- [ ] **VP-001**: Fix MATCH WHERE for LIKE/BETWEEN/IN conditions
  - `where_eval.rs` line 69: `_ => Ok(true)` must handle all condition types
  - Impact: All queries using LIKE '%pattern%' or BETWEEN or IN in MATCH context

- [ ] **VP-002**: Implement subquery execution or return clear error
  - `conversion.rs` line 23-27: Subquery → Value::Null silently
  - Decision needed: implement subqueries OR return UnsupportedFeature error
  - Impact: ALL 4 business scenarios in README

- [ ] **VP-003**: Wire temporal expressions in MATCH WHERE
  - NOW() - INTERVAL parses and converts to epoch seconds
  - MATCH WHERE doesn't evaluate temporal comparisons
  - Impact: Fraud detection, AI Agent Memory scenarios

- [ ] **VP-004**: Multi-hop MATCH pattern execution
  - `(a)-[:R1]->(b)-[:R2]->(c)` — only first relationship used
  - Need proper multi-hop binding propagation
  - Impact: Fraud detection `*1..3`, Healthcare multi-relationship

- [ ] **VP-005**: RETURN clause with aggregation in MATCH
  - `RETURN treatment.name, AVG(success_rate)` — not implemented
  - Impact: Healthcare scenario, Technical deep-dive

- [ ] **VP-006**: ORDER BY property in MATCH (not just similarity)
  - Currently returns UnsupportedFeature error for property ORDER BY
  - Impact: AI Agent Memory scenario `ORDER BY conv.timestamp DESC`

- [ ] **VP-007**: End-to-end integration tests for ALL README scenarios
  - Create test suite that executes every README query example
  - Verify correct results or clear error messages
  - Impact: Prevents future promise-reality drift

- [ ] **VP-008**: README accuracy audit and correction
  - Remove or annotate features that don't work yet
  - Add "Coming Soon" labels where appropriate
  - Update business scenario queries to use actually-working syntax

- [ ] **VP-009**: Website/screenshot claims audit
  - Verify each website card claim (Agentic Memory, GraphRAG, etc.)
  - Ensure code examples in screenshots actually execute

### Out of Scope

- **New features not promised** — Only fix what's already claimed
- **Performance optimization** — Correctness first
- **Premium features** (CRDT sync, Multi-Agent) — Correctly labeled as Premium
- **Distributed mode** — Correctly labeled as Planned v1.5
- **velesdb-python crate** — Listed in README but exists in workspace

## Constraints

- **Zero false advertising** — Every claim must be backed by working code or labeled "Coming Soon"
- **Clear errors over silent failures** — UnsupportedFeature errors are acceptable; Value::Null is not
- **Backward compatibility** — Don't break existing working queries
- **TDD** — Tests BEFORE implementation for every fix
- **Quality gates** — All must pass: fmt, clippy, deny, test

## Key Decisions

| Decision | Options | Rationale | Outcome |
|----------|---------|-----------|---------|
| Subqueries | Implement vs Error | User chose: Implement. Delivers on README promise. | **Implement** ✅ |
| Business scenarios | Fix queries vs Update README | Both needed — fix what's feasible, update docs for rest | — Pending |
| Multi-hop MATCH | Full Cypher vs Simplified | Simplified multi-hop covers 90% of use cases | — Pending |
| RETURN aggregation | Full vs Property-only | Property projection works; aggregation deferred | — Pending |

---
*Created: 2026-02-08*
*Audit basis: codebase at commit on v2-core-trust branch*
