# VelesQL in the browser (WASM)

Companion to [`crates/velesdb-wasm/README.md`](../../crates/velesdb-wasm/README.md).
VelesQL is **parsed, validated and executed** entirely client-side by
`@wiscale/velesdb-wasm`. This guide states exactly what runs, what is rejected,
and when to move to the REST server instead.

## Parsing and validation

`VelesQL` is a static class — nothing to construct.

```javascript
import init, { VelesQL } from '@wiscale/velesdb-wasm';

await init();

const parsed = VelesQL.parse('SELECT * FROM docs WHERE vector NEAR $v LIMIT 10');
console.log(parsed.tableName);        // "docs"
console.log(parsed.hasVectorSearch);  // true
console.log(parsed.limit);            // 10n   <- BigInt, not 10

console.log(VelesQL.isValid('SELECT * FROM docs'));   // true
console.log(VelesQL.isValid('SELEC * FROM docs'));    // false

const match = VelesQL.parse('MATCH (p:Person)-[:KNOWS]->(f:Person) RETURN f.name');
console.log(match.isMatch);                 // true
console.log(match.matchNodeCount);          // 2
console.log(match.matchRelationshipCount);  // 1
```

`ParsedQuery` also exposes `isValid`, `isSelect`, `isDdl`, `isDml`,
`collectionName`, `columns`, `hasDistinct`, `hasWhereClause`, `hasOrderBy`,
`hasGroupBy`, `hasJoins`, `hasFusion`, `offset`, `orderBy`, `groupBy` and
`joinCount` — all **properties**, not calls. `limit` and `offset` are
`bigint | undefined`.

## Executing queries

Execution goes through `WasmDatabase.executeQuery(sql, paramsJson)`. The second
argument is a **JSON string** (or `null`), not an object. The result is a
`QueryResult` with `kind`, `message`, `rowCount` and `rowsJson`.

```javascript
import init, { WasmDatabase } from '@wiscale/velesdb-wasm';

await init();

const db = new WasmDatabase();
db.createMetadataCollection('docs');

let r = db.executeQuery("INSERT INTO docs (id, title, views) VALUES (1, 'hello', 5)", null);
console.log(r.kind, r.rowCount, r.message);   // mutation 1 1 row(s) affected

db.executeQuery("INSERT INTO docs (id, title, views) VALUES (2, 'world', 50)", null);

r = db.executeQuery('SELECT title FROM docs WHERE views > 10', null);
console.log(r.kind, r.rowCount, r.rowsJson);  // rows 1 [{"title":"world"}]
```

`EXPLAIN` uses the same plan vocabulary as the REST server (core's
`QueryPlan::to_plan_steps()`), so a plan captured in the browser is comparable
to a server plan:

```javascript
const plan = db.executeQuery('EXPLAIN SELECT title FROM docs WHERE views > 10', null);
console.log(plan.rowsJson);
// [{"description":"Scan collection 'docs'","estimated_rows":2,
//   "estimation_method":"row count","operation":"FullScan","step":1},
//  {"description":"Apply WHERE clause predicates","operation":"Filter","step":2}, …]
```

`createMetadataCollection` exists precisely so payload-only workloads can run
VelesQL without declaring a `vector` column.

## What runs in WASM

| Feature | WASM | REST server |
|---|---|---|
| Vector search (`NEAR`) | yes | yes |
| Metadata filtering | yes | yes |
| Hybrid search (vector + text) | yes | yes |
| Full-text search | yes | yes |
| Multi-query fusion (MQG) | yes | yes |
| Batch search | yes | yes |
| Sparse search | yes | yes |
| Knowledge graph (nodes, edges, traversal) | yes | yes |
| Agent memory (`MemoryService`) | yes | yes |
| VelesQL parsing and validation | yes | yes |
| VelesQL execution | yes, minus the carve-outs below | yes |
| Column projection / aliases / window functions | yes | yes |
| `GROUP BY` / `HAVING` / aggregates | yes | yes |
| Aggregate `ORDER BY` over a `GROUP BY` | yes | yes |
| `UNION` / `INTERSECT` / `EXCEPT` | yes | yes |
| `JOIN` | `INNER`, `LEFT` only | all |
| `MATCH` graph traversal | 1–2 hops | any depth |
| Cross-collection `MATCH` (`@collection`) | no | yes |
| `EXPLAIN` (core plan vocabulary) | yes | yes |
| Persistence | IndexedDB | Disk (mmap) |
| Practical ceiling | ~100 K vectors (browser RAM) | millions |

The single-collection executor supports `SELECT` (with `WHERE`, `NEAR`,
`similarity()`), projection / aliases / window functions
(`ROW_NUMBER`, `RANK`, `DENSE_RANK`), `GROUP BY` / `HAVING`, aggregates,
`ORDER BY` (payload columns, `similarity()`, arithmetic expressions, and
aggregate `ORDER BY` over a `GROUP BY`), a default `LIMIT 10`,
`UNION` / `INTERSECT` / `EXCEPT`, `INNER` / `LEFT JOIN`,
`INSERT` / `UPSERT` / `UPDATE` / `DELETE`, DDL
(`CREATE` / `DROP` / `TRUNCATE COLLECTION`), introspection
(`SHOW COLLECTIONS`, `DESCRIBE COLLECTION`), admin (`FLUSH`, a no-op), and
1–2 hop `MATCH`.

## What is rejected, and how

Every unsupported shape is a **loud rejection**. WASM never returns a
quietly-wrong result for a query it cannot honour. The messages below are a
historical reference from `@wiscale/velesdb-wasm@4.0.0`.

| Shape | Message |
|---|---|
| `LET` score bindings | `LET bindings are not supported in WASM` |
| Scalar subqueries | `Subqueries are not supported in WASM` |
| `RIGHT JOIN` | `RIGHT JOIN is not supported in WASM (use LEFT JOIN)` |
| `FULL JOIN` | `FULL JOIN is not supported in WASM` |
| `MATCH` beyond 2 hops | `MATCH patterns with more than 2 hops are not yet supported in WASM (N nodes)` |
| `ALTER COLLECTION` | `ALTER COLLECTION is not supported in WASM yet` |
| Graph collections in DDL | `Graph collections are not supported in WASM (use GraphStore directly)` |
| `similarity()` threshold in `WHERE` | `similarity() threshold filters are not supported in WASM` |
| Graph `MATCH` predicate in `WHERE` | `Graph MATCH predicates are not supported in WASM` |
| BM25 `MATCH` condition in `WHERE` | `MATCH (BM25) conditions are not supported in WASM` |
| `CONTAINS` / `CONTAINS_TEXT` | `CONTAINS / CONTAINS_TEXT conditions are not supported in WASM` |
| Geospatial conditions | `Geospatial conditions are not supported in WASM` |
| Inline vectors in `INSERT` | `… inline vectors are not supported in WASM INSERT` |
| `ORDER BY similarity(field, $v)` | `ORDER BY similarity(field, $vec) is not supported in WASM: named/secondary …` |
| `ORDER BY` in a `MATCH` | `MATCH ORDER BY <form> is not supported in WASM (use depth or alias.property)` |

Two cases deserve more than a one-liner:

- **Cross-collection `MATCH` (`@collection`)** — the `@` form is not even in
  the WASM grammar; it fails at parse time (`expected identifier`), not at
  execution. It requires Database-level query routing that only the server has.
- **`TRAIN QUANTIZER`** — recognized for API parity but unavailable: training
  needs `ndarray`/`persistence`, which are compiled out for WASM. Product
  Quantization has the same constraint (it needs `rayon`).

### Fusion strategies

`USING FUSION(strategy=…)` behaves differently depending on the query shape:

- On a **single-vector `NEAR`**, WASM has no BM25 or graph branch to fuse
  against, so weight-sensitive strategies (`weighted`, `rsf`) are meaningless.
  Use `rrf`, `maximum` or `average`, or a plain metadata filter.
- On a **multi-vector `NEAR_FUSED`** query the strategy is not rejected, but
  only `rrf`, `average` and `maximum` are honoured — `weighted` and `rsf`
  fall back to RRF, matching core's `fused_config_to_strategy`.

### `ORDER BY similarity()`

WASM stores only the **primary** vector, so the named/secondary form
`ORDER BY similarity(field, $v)` is rejected on both the `SELECT` and `MATCH`
paths. Use `ORDER BY similarity()` (the search score itself) or a payload
column. The `MATCH` path does no vector scoring at all, so it additionally
rejects bare `similarity()` and arithmetic `ORDER BY` — order by `depth` or
`alias.property` there.

## Error surface: mixed, coerce before reading

In the historical 4.0.0 release, error reporting was **partly structured**.
Some boundaries threw a real
JS `Error` carrying a non-enumerable, machine-readable `code`; others still
threw a bare string. This historical reference was verified against
the historical `@wiscale/velesdb-wasm@4.0.0` package:

| Operation | Thrown value |
|---|---|
| `store.search()` with a wrong query length | `Error`, `e.code === 'VELES-004'`, message `[VELES-004] Vector dimension mismatch: expected 3, got 2` |
| `new VectorStore(3, 'nope')` | plain string, `Unknown metric. Use: cosine, euclidean, l2, dot, dotproduct, inner, ip, hamming, jaccard` |
| `db.get_collection('nope')` | plain string, `Collection 'nope' not found` |
| `db.executeQuery('SELECT * FROM missing')` | plain string, `Collection 'missing' not found` |
| A VelesQL syntax error | plain string, `VelesQL parse error at position N: …` with a caret-annotated excerpt |

Write handlers that survive both:

```javascript
try {
  db.executeQuery(sql, null);
} catch (e) {
  const code = e && typeof e === 'object' ? e.code : undefined;   // may be undefined
  console.error(code ?? 'UNKNOWN', String(e));
}
```

## When to move to the REST server

Reach for the [REST server](https://github.com/cyberlife-coder/VelesDB) when you
need:

- **Cross-collection `MATCH`** — `@collection` routing lives on the server.
- **Multi-hop `MATCH`** — traversals beyond 2 hops.
- **More than ~100 K vectors** — browser RAM is the ceiling.
- **`RIGHT`/`FULL JOIN`, quantizer training, geospatial or BM25 predicates.**
- **Centralized, shared state** — WASM is per-tab and per-user by definition.

### Migrating a query to REST

```javascript
// Client-side (WASM)
import init, { VectorStore } from '@wiscale/velesdb-wasm';
await init();
const store = new VectorStore(768, 'cosine');
const wasmResults = store.search(query, 10);

// Server-side (REST) — vector search
const searchResponse = await fetch('http://localhost:8080/collections/docs/search', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ vector: Array.from(query), top_k: 10 }),
});
const restResults = await searchResponse.json();

// Server-side (REST) — VelesQL
const queryResponse = await fetch('http://localhost:8080/query', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    query: "SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' LIMIT 10",
    params: { v: Array.from(query) },
  }),
});
const restRows = await queryResponse.json();
```

The REST endpoints above are the server's documented surface; check
[`docs/openapi.yaml`](../openapi.yaml) for the authoritative contract of the
version you run.

## Related

- [VelesDB WASM JavaScript API](WASM_API.md)
- [VelesDB WASM persistence and binary format](WASM_PERSISTENCE.md)
- [VelesQL specification](../VELESQL_SPEC.md)
- [Error codes reference](../reference/ERROR_CODES.md)

---

Last updated: 2026-08-13 · Applies to: velesdb-core 6.0.0
