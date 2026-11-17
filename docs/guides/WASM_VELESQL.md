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

### Graph patterns: `executeMatch`

A `MATCH` reads one graph: the `_collection` param, else the default `graph`
(MATCH has no `FROM`). As in core, the points of the collection with that name
are its nodes (payload as properties, `_labels` as labels), and its edges come
from `INSERT EDGE`. The walk is core's:

- a pattern node needs **every** label it lists, a relationship **any** type it
  lists (`[:A|B]`);
- inline properties `(n {k: v})` and `-[r {k: v}]->` filter as `n.k = v` would
  in WHERE (three-valued, a `$name` bound from the params, `id` the element id);
- an alias repeated in the pattern names one node (`(a)-[:R]->(b)-[:R]->(a)` is
  a cycle), and an unnamed node is walked but bound to no name;
- `WHERE r.prop` reads a named relationship's properties, and a bare column
  reads the walk's target, the last node;
- one row per distinct binding of the named nodes and relationships;
- `(w:Warehouse@pricing)` gives `w` the properties of the `pricing` point with
  the same id (the node's own win), seen by WHERE, the pattern, ORDER BY and
  RETURN alike; an annotation naming no collection refuses the MATCH.

An `executeQuery` MATCH row carries the walk's target as its id and, as its
score, the `similarity()` score or else `1 / (hops + 1)`, as core's rows do.

`executeQuery` returns one alias-keyed object per MATCH row. `executeMatch`
runs the same MATCH and returns a JSON string of rows in core's
`MatchResult` shape, the rows of REST `POST /collections/{name}/match`:

```javascript
// Edges as data (like REST /graph/edges); VelesQL INSERT EDGE takes literals only.
db.addEdge('docs', JSON.stringify({ source: 1, target: 2, label: 'CITES', properties: { weight: 0.5 } }));

const rows = JSON.parse(db.executeMatch(
  "MATCH (a)-[:CITES]->(b) RETURN b.title AS cited, b.id",
  JSON.stringify({ _collection: 'docs' })
));
// [{ "bindings": { "a": "1", "b": "2" }, "depth": 1,
//    "projected": { "cited": "world", "b.id": 2 } }]
```

Each `bindings` node id is its exact decimal string, as REST `/match` sends it
(a JSON number rounds a `u64` past 2^53). `projected` keeps core's VelesQL row
contract, shared with `executeQuery` and REST `/query`: an `alias.id` there is
a number.

`projected` follows core's RETURN rules: `alias.property` under its `AS` alias or
its text, `alias.id` for the element id, a bare alias as every `alias.key` plus
`alias.id`, and `*` for every bound node.

The nodes' vectors score `similarity()` as in core. `WHERE similarity(b.embedding,
$v) > 0.8` scores the node `b` binds (a bare field: the row's last node), and
under a distance metric "more similar" means "closer". `ORDER BY similarity()`
is most-similar-first and `RETURN similarity()` projects the score, which the
row also carries as `score`. `executeMatch(sql, params, vector, threshold)`
takes REST `/match`'s query vector and threshold: it scores each row's last
node, drops rows past the threshold and orders by score.

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
| `JOIN` | `INNER`, `LEFT`, `RIGHT`, `FULL`, on any column | all |
| `MATCH` graph traversal | one fixed-length pattern, any number of hops | any depth, multi-pattern, variable-length |
| Cross-collection `MATCH` (`@collection`) | yes | yes |
| `EXPLAIN` (core plan vocabulary) | yes | yes |
| Persistence | IndexedDB | Disk (mmap) |
| Practical ceiling | ~100 K vectors (browser RAM) | millions |

The single-collection executor supports `SELECT` (with `WHERE`, `NEAR`,
`NEAR_FUSED`, `similarity()` thresholds — on the point's own vector or a named
payload vector, several under `AND` — read and ranked as in core),
`SELECT DISTINCT` (core's dedup key, applied before `ORDER BY` and `LIMIT`),
projection / aliases / window functions
(`ROW_NUMBER`, `RANK`, `DENSE_RANK`), `GROUP BY` / `HAVING`, aggregates,
`ORDER BY` (payload columns, `similarity()`, `score` and the component scores
resolved by core's `velesql::ScoreContext`, arithmetic expressions, and
aggregate `ORDER BY` over a `GROUP BY`), a default `LIMIT 10`,
`UNION` / `INTERSECT` / `EXCEPT` (core's semantics, `velesql::execute_compound`;
an operand cannot join or aggregate), `INNER` / `LEFT` / `RIGHT` / `FULL JOIN`
on any column (core's algorithm, `velesql::hash_join`, and core's joined row,
each side under its alias; WHERE, DISTINCT, ORDER BY, GROUP BY and LIMIT over
the joined rows),
`INSERT` / `UPSERT` / `UPDATE` / `DELETE`, DDL
(`CREATE` / `DROP` / `TRUNCATE COLLECTION`), introspection
(`SHOW COLLECTIONS`, `DESCRIBE COLLECTION`), admin (`FLUSH`, a no-op), and
fixed-length `MATCH` of any number of hops.

A `WHERE` predicate (comparisons, `IN`, `BETWEEN`, `LIKE` / `ILIKE`,
`IS NULL`, through `AND` / `OR` / `NOT`) is evaluated by core's payload
filter, the one the server runs: its parameters bind as in core and it
answers the same rows — `x != 'a'` and `x NOT IN (…)` hold on a payload
without `x`, `LIKE` honours `\` escapes, numbers compare by exact value.

## Parity with core, family by family

The executor is WASM's own (core's needs the `persistence` feature), so its
answers are checked rather than assumed: every family below runs the same
query text on both engines from `conformance/velesql_executor_cases.json`,
which asserts ids, order, scores and refusals
(`crates/velesdb-wasm/src/velesql_executor_conformance_tests.rs` beside
`crates/velesdb-core/tests/velesql_executor_conformance.rs`). Both engines run
core's `QueryValidator` first, so a refusal carries the same `[Vnnn]` code.

| Family | WASM vs core | Pinned by |
|---|---|---|
| `WHERE`: comparisons, `IN`, `BETWEEN`, `LIKE` / `ILIKE`, `IS NULL`, `AND` / `OR` / `NOT` | Same rows — core's payload filter runs in both | `cases` X001–X020, `number_cases`, `join_cases` J015 (`IS NULL`) |
| `NEAR`, `similarity()` thresholds, `NEAR` under `OR` | Same ids and scores | `near_cases` N001–N034, `cases` X021–X022 |
| `NEAR_FUSED` + `USING FUSION` `rrf` / `average` / `maximum` | Same ids and scores; `weighted` / `rsf` refused by both (V012) | `fusion_cases`, `rejected_cases` |
| `SPARSE_NEAR` alone, or beside a `NEAR` with or without `USING FUSION` | Same ids and scores | `velesql_sparse_parity_tests.rs` (against a core `Database`) |
| `ORDER BY`, `DISTINCT`, `LIMIT` / `OFFSET`, nulls ordering | Same order; a NaN score ranks last, as in core | `cases`, `null_order_cases`, `near_cases`; NaN: `vector_ops_tests.rs`, `velesql_match_rank_tests.rs` |
| Projection, aliases, window functions | Same rows | `projection_cases` P001–P009 |
| `GROUP BY` / `HAVING` / aggregates, vector `GROUP BY` | Same groups and values | `aggregate_cases` G001–G027, `vector_group_cases` V001–V010 |
| `UNION` / `INTERSECT` / `EXCEPT` | Same ids, in order | `setops_cases` S001–S008 |
| `JOIN` (`INNER` / `LEFT` / `RIGHT` / `FULL`, any column) | Same joined rows | `join_cases` J001–J018 |
| `INSERT` / `UPSERT` / `UPDATE` / `DELETE` | Same effect | `dml_cases` W001–W010 |
| Fixed-length `MATCH`: hops, anonymous nodes, inline properties, `WHERE`, `ORDER BY` | Same rows; edges are stored beside the collection rather than on it (D004, storage only) | `match_cases` M001–M006; labels, relationship properties, repeated aliases and `@collection` against core's semantics in `velesql_match_parity_tests.rs` |
| Shapes core's validator refuses | Same `[Vnnn]` code | `rejected_cases` R001–R017 |
| `NOW()` / `INTERVAL` | Same — `NOW()` reads the page's clock | `crates/velesdb-wasm/tests/velesql_temporal_web.rs` |

What WASM does **not** run, refused by name (the next section lists the
messages): multi-pattern and variable-length `MATCH`, scalar subqueries, `LET`,
BM25 `MATCH` / `CONTAINS` / geospatial predicates in `WHERE`,
`ALTER COLLECTION`, graph collections in DDL and `TRAIN QUANTIZER`. A
weight-sensitive `USING FUSION` beside a single `NEAR` is refused too (D013):
its only second branch in WASM would be synthetic. One divergence is silent:
`WITH (mode = …, ef_search = …)` is not read, since WASM searches exactly by
brute force and has no HNSW recall to tune.

Outside VelesQL, two JavaScript entry points differ from core by design:
`VectorStore.textSearch` is a case-insensitive substring match with no score,
not BM25, and `hybrid_search` blends that match linearly with the dense score
(expected divergence F003 in `conformance/search_fidelity_cases.json`).

## What is rejected, and how

Every query first goes through `velesdb-core`'s own `QueryValidator`, the one
the server and the CLI run, so a query core refuses is refused here with the
same `[Vnnn]` message rather than answered differently.

Every unsupported shape is a **loud rejection**. WASM never returns a
quietly-wrong result for a query it cannot honour, with one exception: the
executor reads no `WITH` option (`mode`, `ef_search`, `rerank`, …), so a query
carrying one runs as if it had none. The messages below are a historical
reference from `@wiscale/velesdb-wasm@4.0.0`.

| Shape | Message |
|---|---|
| `LET` score bindings | `LET bindings are not supported in WASM` |
| Scalar subqueries | `Subqueries are not supported in WASM` |
| Multi-pattern `MATCH` | `Multi-pattern MATCH is not yet supported in WASM (N patterns in this query)` |
| Variable-length relationship (`[*1..3]`) | `Variable-length relationships (e.g. [*1..3]) are not yet supported in WASM` |
| `ALTER COLLECTION` | `ALTER COLLECTION is not supported in WASM yet` |
| Graph collections in DDL | `Graph collections are not supported in WASM (use GraphStore directly)` |
| Graph `MATCH` predicate in `WHERE` | `Graph MATCH predicates are not supported in WASM` |
| BM25 `MATCH` condition in `WHERE` | `MATCH (BM25) conditions are not supported in WASM` |
| `CONTAINS` / `CONTAINS_TEXT` | `CONTAINS / CONTAINS_TEXT conditions are not supported in WASM` |
| Geospatial conditions | `Geospatial conditions are not supported in WASM` |
| Inline vectors in `INSERT` | `… inline vectors are not supported in WASM INSERT` |
| An aggregate `ORDER BY` in a `MATCH` | `MATCH ORDER BY aggregate expression is not supported …` (as in core) |

One case deserves more than a one-liner:

- **`TRAIN QUANTIZER`** — recognized for API parity but unavailable: training
  needs `ndarray`/`persistence`, which are compiled out for WASM. Product
  Quantization has the same constraint (it needs `rayon`).

### Fusion strategies

`USING FUSION` follows core's validation (V012):

- On a **single-vector `NEAR`** there is one fusable branch and nothing to
  fuse, so `USING FUSION` is refused whatever the strategy. Drop the clause:
  a metadata predicate beside the `NEAR` already filters the hits.
- On a **multi-vector `NEAR_FUSED`** query, `rrf` (the default, with an
  optional `k`), `average` and `maximum` run exactly as in core, scores
  included; `weighted` and `rsf` are refused, being ill-defined over
  homogeneous query vectors. Each query vector contributes the same number of
  best points passing the metadata filter as in core
  (`fusion::branch_depth`), at most `fusion::MAX_FUSED_VECTORS` vectors, and a
  distance metric fuses closest-first.
- A **`NEAR` beside a `SPARSE_NEAR`** is core's hybrid dense + sparse search:
  each branch keeps as many best points passing the metadata filter as core
  (`fusion::dense_sparse_candidate_k`), and the branches fuse by the strategy
  core maps from the clause (`FusionClause::dense_sparse_strategy`, RRF without
  one): `rrf`, `average`, `maximum`, `rsf`, `weighted` and `weighted_rrf` all
  run, with a distance metric read closest-first. A lone `SPARSE_NEAR` ranks
  by the sparse score. The store's one sparse index is the default index: a
  store without sparse vectors, or a named index (`USING 'name'`), is refused
  as core refuses a missing index, with core's error.
- `DISTINCT` and a vector `GROUP BY` beside a `SPARSE_NEAR` or a
  `NEAR_FUSED` read every candidate core reads before `LIMIT` / `OFFSET`
  apply (`SelectStatement::reshapes_rows_before_window`), so the window bounds
  the distinct rows or groups, not the hits.
- A `similarity()` threshold beside a `SPARSE_NEAR` bounds both branches,
  evaluated as on the dense path, under `AND`, `OR` and `NOT`, as core
  bounds them.
- The vector-predicate shapes core refuses (`NEAR_FUSED` under `OR` / `NOT` or
  beside another vector predicate, several `SPARSE_NEAR`, several
  `similarity()` under one `OR`) are refused by the same check
  (`velesql::validate_vector_predicates`).

Both rules, and the ranking of every accepted shape, are pinned against core by
`conformance/velesql_executor_cases.json` (`fusion_cases`, `rejected_cases`).

### `ORDER BY similarity()`

`ORDER BY similarity()` is the search score. `ORDER BY similarity(field, $v)`
scores each row as core does: `vector` against the row's own vector, any other
field against the payload's named vector, a row without one of the query's
dimension taking the worst key (last under `DESC`). It is refused only over
grouped rows. A `MATCH` orders as core's does: `depth`, `alias.property`,
`similarity()`, `similarity(alias.vector, $v)` (the node `alias` binds, the
target for a bare field) and arithmetic over the target.

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

- **Multi-pattern or variable-length `MATCH`** — `(a), (b)` patterns and `[*1..3]` ranges.
- **More than ~100 K vectors** — browser RAM is the ceiling.
- **Quantizer training, geospatial or BM25 predicates.**
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

Last updated: 2026-10-29 · Applies to: velesdb-core 8.2.0
