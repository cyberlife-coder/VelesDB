# Phase v3-03: TypeScript SDK — Full Parity & Quality — Context

**Captured:** 2026-02-10

## Vision

The TypeScript SDK must be a **faithful, complete binding** to everything velesdb-core exposes via the REST server. No missing endpoints. No half-implemented contracts. No `any` casts. Every file under 300 lines. Every public method tested.

Today the SDK covers 24/25 server endpoints, but has structural debt (856-line monolith), missing features (EXPLAIN, search tuning), and no SELECT query builder. This phase finishes the job.

## Scope — Full Gap Closure

### What Plan 01 Already Fixed (✅ Done)

- `search()` response unwrapping (T-01)
- `listCollections()` response mapping (T-02)
- `delete()` default on success (T-03)
- `query()` aggregation detection + smart MATCH routing
- `matchQuery()` for MATCH endpoint
- Promise-based init lock (BEG-07)
- Type safety sweep (zero `any`, zero `!`)

### What Remains

| Gap | Severity | Description |
|-----|----------|-------------|
| **EXPLAIN endpoint** | High | `POST /query/explain` — query plan, cost estimation, feature detection. Zero SDK support. |
| **search() ef_search/mode** | Medium | Server supports HNSW recall tuning (`ef_search`, `mode` presets). SDK doesn't expose them. |
| **Types in wrong location** | Medium | MATCH types local to `rest.ts` instead of shared `types.ts`. `IVelesDBBackend` missing `explain()`. |
| **rest.ts 856 lines** | High | Single monolith violates <300 line rule. Unmaintainable. Must extract into modules. |
| **No SELECT query builder** | High | `VelesQLBuilder` only handles MATCH. No way to build SELECT/aggregation/JOIN queries fluently. |
| **VelesQLBuilder gaps** | Medium | Missing: SELECT mode, FROM, GROUP BY, aggregation functions, JOIN, HAVING. |
| **Test coverage gaps** | Medium | New methods (explain, select builder) need full TDD coverage. |

## User Experience

A developer using the SDK should be able to:

```typescript
// Every server feature accessible from one import
import { VelesDB, velesql, selectql } from '@velesdb/sdk';

const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
await db.init();

// Vector search with recall tuning
const results = await db.search('docs', embedding, {
  k: 20, efSearch: 200, mode: 'accurate'
});

// EXPLAIN before executing
const plan = await db.explain('docs',
  'SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 LIMIT 10'
);
console.log(plan.queryType, plan.estimatedCost.complexity);

// Fluent SELECT builder
const { query, params } = selectql()
  .from('products')
  .select('name', 'price')
  .where('category = $cat', { cat: 'electronics' })
  .orderBy('price', 'DESC')
  .limit(10)
  .build();

const response = await db.query('products', query, params);

// Fluent MATCH builder (existing)
const matchQuery = velesql()
  .match('a', 'Person').rel('KNOWS').to('b', 'Person')
  .similarity('a', '$v', embedding, { threshold: 0.8 })
  .return(['a.name', 'b.name'])
  .toVelesQL();
```

## Essentials

Things that MUST be true:

- **100% endpoint coverage** — Every server route has a corresponding SDK method
- **`explain()` fully implemented** — Types, interface, REST backend, client, WASM stub
- **All files < 300 lines** — `rest.ts` (856) extracted into cohesive modules
- **Zero `any` casts** in production code (`rest.ts`, `client.ts`, `types.ts`)
- **Zero `!` non-null assertions** without safe fallback
- **SELECT query builder** — Fluent API for building SELECT queries (not just MATCH)
- **SearchOptions extended** — `efSearch` and `mode` for HNSW tuning
- **TDD** — Tests written before implementation for all new features
- **`IVelesDBBackend` updated** — Every new method in the interface contract
- **All 6 test files pass** — `npx tsc --noEmit && npx vitest run` green

## Boundaries

Things to explicitly AVOID:

- **Do NOT touch WASM backend internals** — That's Phase v3-01 scope. Only add stubs/throws for new interface methods.
- **Do NOT add runtime dependencies** — SDK must stay zero-dep (only dev deps for testing)
- **Do NOT change public API signatures** of existing methods — Only ADD new methods or EXTEND options
- **Do NOT over-abstract** — No class hierarchies, no DI frameworks. Keep it simple: one class per file, clear exports.
- **Do NOT implement streaming/websocket** — That's a future feature. `stream` option stays in types but is not implemented.
- **Do NOT touch server code** — This phase is SDK-only

## Implementation Notes

### rest.ts Extraction Strategy (Expert Recommendation)

Split `rest.ts` (856 lines) into:

```
sdks/typescript/src/backends/rest/
├── index.ts          # Re-exports RestBackend (facade)
├── base-client.ts    # HTTP client, init, request(), error handling (~120 lines)
├── collections.ts    # createCollection, getCollection, listCollections, deleteCollection, isEmpty, flush (~100 lines)
├── points.ts         # insert, insertBatch, get, delete (~80 lines)
├── search.ts         # search, searchBatch, multiQuerySearch, textSearch, hybridSearch (~120 lines)
├── query.ts          # query, matchQuery, explain (~120 lines)
├── graph.ts          # addEdge, getEdges, traverseGraph, getNodeDegree (~100 lines)
└── types.ts          # Server-internal interfaces (ServerMatchQueryResponse, etc.) (~50 lines)
```

Each file uses composition: imports base client, exports method implementations.
RestBackend class stays as facade, delegates to module functions.

### SELECT Query Builder

New `SelectBuilder` class (separate from `VelesQLBuilder` which is MATCH-specific):

```typescript
selectql()
  .from('products')
  .select('name', 'price')           // or .selectAll()
  .selectAgg('COUNT', '*')           // aggregation
  .where('price > $min', { min: 10 })
  .andWhere('category = $cat', { cat: 'tech' })
  .nearVector('$v', embedding, { topK: 20 })
  .groupBy('category')
  .orderBy('price', 'DESC')
  .limit(10)
  .offset(20)
  .build()  // → { query: string, params: Record<string, unknown> }
```

### EXPLAIN Types

```typescript
interface ExplainResponse {
  query: string;
  queryType: 'SELECT' | 'MATCH';
  collection: string;
  plan: ExplainStep[];
  estimatedCost: ExplainCost;
  features: ExplainFeatures;
}
```

## Plan Structure (5 plans, 3 waves)

### Wave 1 — API Surface Completion (parallel)

| Plan | Title | Scope | Est. |
|------|-------|-------|------|
| **02** | Types Export + VelesQLBuilder MATCH | Move types to types.ts, add to IVelesDBBackend, similarity() | 30min |
| **03** | EXPLAIN Endpoint + Search Options | explain() full stack, SearchOptions.efSearch/mode | 30min |

### Wave 2 — Structural Quality (sequential, depends on Wave 1)

| Plan | Title | Scope | Est. |
|------|-------|-------|------|
| **04** | rest.ts Module Extraction | Split 856-line monolith → 7 files, all < 300 lines | 45min |

### Wave 3 — DX & Coverage (parallel, depends on Wave 2)

| Plan | Title | Scope | Est. |
|------|-------|-------|------|
| **05** | SELECT Query Builder | SelectBuilder fluent API + TDD tests | 30min |
| **06** | Test Coverage Sweep | Fill gaps, edge cases, integration patterns | 20min |

**Total estimated: ~2.5 hours**

## Open Questions

Things to decide during planning:

- **SelectBuilder naming**: `selectql()` factory or extend existing `velesql()` with `.select().from()` syntax?
- **rest.ts extraction**: Use class mixin pattern or standalone functions with injected client?
- **EXPLAIN in WASM**: Stub that throws, or could WASM do client-side plan estimation?

## Out of Scope (flagged for other phases)

- **WASM backend reimplementation** → Phase v3-01 (WASM Rebinding)
- **wasm.ts 518 lines + `any` casts** → Phase v3-01
- **Server authentication** → Phase v3-02 (Server Binding & Security)
- **Streaming responses** → Future feature
- **Python SDK** → Phase v3-04

---
*This context informs planning. The planner will honor these preferences.*
