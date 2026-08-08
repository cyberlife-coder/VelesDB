# VelesQL REPL reference

Complete reference for the interactive REPL started by
`velesdb repl [path]`, shipped by the
[`velesdb-cli`](../../crates/velesdb-cli/README.md) crate.

Related: [CLI command reference](CLI_COMMAND_REFERENCE.md) ·
[VelesQL cookbook](CLI_VELESQL_COOKBOOK.md) ·
[VelesQL spec](../VELESQL_SPEC.md)

---

## Starting the REPL

```bash
velesdb repl            # default database directory: ./data
velesdb repl ./my_db    # explicit path
```

```
VelesDB v4.3.0 - VelesQL REPL
Database: ./data
Type .help for commands, .quit to exit

velesdb>
```

The REPL accepts three kinds of input:

1. **dot-commands** — `.help`, `.schema docs`, `.graph count kg`
2. **backslash session commands** — `\set`, `\show`, `\use` (the dot prefix is
   also accepted: `.set`, `.show`, `.use`)
3. **raw VelesQL** — anything else is parsed, validated and executed

Two behaviours worth knowing up front:

- The REPL is **single-line only** — each command or query must fit on one line.
  Multi-line input is not supported.
- History is persisted across sessions in `.velesdb_history` inside the
  platform local-data directory (`~/.local/share/.velesdb_history` on Linux),
  falling back to the current directory when no such directory is available.

`Ctrl-C` prints `Use .quit to exit`; `Ctrl-D` exits.

---

## General commands

| Command | Aliases | Description |
|---------|---------|-------------|
| `.help` | `.h` | Show all available commands |
| `.quit` | `.exit`, `.q` | Exit the REPL |
| `.collections` | `.tables` | List all collections (vector, graph, metadata) |
| `.clear` | | Clear the terminal screen |
| `.timing on\|off` | | Toggle query execution time display (default: on). Also accepts `true`/`false`, `1`/`0`. |
| `.format table\|json` | | Set output format for query results |

## Collection inspection

| Command | Aliases | Description |
|---------|---------|-------------|
| `.schema <name>` | | Show collection type, dimension, metric, and point count |
| `.describe <name>` | `.desc` | Detailed collection info (memory estimate, schema, storage mode) |
| `.stats <name>` | | Collection statistics (point count, dimension, memory) |
| `.diagnostics <name>` | `.diag` | Collection health snapshot (dimension configured, point count, index health) |
| `.count <name>` | | Show record/edge/item count |
| `.sample <name> [n]` | | Show first N records (default: 5). Works for Vector, Graph, and Metadata collections. |
| `.browse <name> [page]` | | Paginated record browsing (10 per page). Works for Vector, Graph, and Metadata collections. |
| `.scroll <name> [batch_size] [cursor]` | | Cursor-based pagination; prints the next cursor after the rows |
| `.nodes <name> [page]` | | Paginated node browsing for Graph collections (20 per page, includes payload) |

## Data operations

| Command | Description |
|---------|-------------|
| `.upsert <col> <id> <vector> [payload]` | Upsert a point, e.g. `.upsert docs 42 [0.5,0.5,0.5,0.5] {"title":"from repl"}` |
| `.export <name> [file]` | Export collection to JSON (default: `<name>.json`). Supports Vector and Metadata collections. |
| `.delete <name> <id> [id2...]` | Delete points by ID |
| `.flush <name>` | Flush collection data to disk |

Vectors and payloads passed to `.upsert` must contain **no spaces** — the REPL
splits input on whitespace.

## Query analysis

| Command | Description |
|---------|-------------|
| `.explain <query>` | Show the execution plan for a VelesQL query (tree format) |
| `.explain-analyze <query>` | Execute the query and print the plan with actual row counts, per-node timings and cache-reuse counters |
| `.analyze <name>` | Analyze collection: row count, deletion ratio, field stats, index stats |
| `.bench <name> [n] [k]` | Benchmark N random queries with top-k (default: 100 queries, k=10). Also available as `\bench`. |

`.explain-analyze` output:

```
Query Plan:
├─ TableScan: quickstart
└─ Limit: 1

Estimated cost: 2.001ms
Cache hit: false
Plan reuse count: 0

Actual Statistics:
  Actual rows: 1
  Actual time: 1.371ms
  Loops: 1
  Nodes visited: 0
  Edges traversed: 0
  Calibration source: N/A
  Cost factors: N/A

Per-Node Statistics:
  TableScan:  1.357ms (rows: 1 → 1) (estimated)
  Limit:  0.014ms (rows: 1 → 1) (estimated)
```

## Index management

| Command | Description |
|---------|-------------|
| `.indexes <name>` | List all indexes on a collection (type, cardinality, memory) |
| `.create-index <name> <field> [--type secondary\|property\|range]` | Create an index (default: secondary) |
| `.drop-index <name> <label> <property>` | Drop an index by label and property |

## Advanced search

| Command | Description |
|---------|-------------|
| `.sparse-search <col> <index> <json> [k]` | Sparse vector search. JSON format: `[[idx, weight], ...]`. Default k=10. |
| `.hybrid-sparse <col> <dense> <sparse> [k] [--strategy rrf\|average\|max] [--index <name>]` | Dense+sparse hybrid search with fusion. Default k=10, strategy=rrf. |
| `.guardrails` | Display current query guard-rails (timeout, memory limit, rate limit, circuit breaker) |
| `.agent [cmd]` | Agent memory commands (preview — not yet fully implemented in the CLI) |

```
velesdb> .sparse-search my_col sparse_idx [[42,0.8],[137,0.6],[891,0.3]] 10
velesdb> .hybrid-sparse docs [0.1,0.2,0.3,0.4] [[0,1.5],[3,0.8]] 10 --strategy rrf
```

Dense vector is a JSON array `[0.1, 0.2, ...]`; sparse vector is
`[[index, weight], ...]`. Valid strategies: `rrf`, `average`, `max` (also
accepts `maximum`).

## Graph commands

| Command | Description |
|---------|-------------|
| `.graph add-edge <col> <id> <src> <tgt> <label>` | Add a directed edge |
| `.graph edges <col> [--label <label>]` | List edges, optionally filtered by label |
| `.graph degree <col> <node_id>` | Show in-degree, out-degree, and total degree |
| `.graph traverse <col> <source> [--algo bfs\|dfs] [--depth N] [--limit N]` | BFS/DFS traversal from a source node |
| `.graph neighbors <col> <node_id> [--direction in\|out\|both]` | List neighbors of a node (default: out) |
| `.graph remove-edge <col> <edge_id>` | Remove an edge by ID |
| `.graph count <col>` | Show edge and node count |
| `.graph search <col> <vector_json> [k]` | Search by embedding similarity (requires embeddings) |
| `.graph store-payload <col> <node_id> <json>` | Store JSON payload on a node |
| `.graph get-payload <col> <node_id>` | Retrieve node payload |
| `.graph nodes <col> [--page N]` | Paginated node browsing (20 per page) |
| `.graph help` | Full graph help |

> **Naming note:** the REPL uses `.graph edges` while the CLI subcommand uses
> `graph get-edges`. Both do the same thing.

> **CLI-only:** `graph doctor` (legacy phantom-edge audit/repair) is a
> standalone CLI subcommand and has no REPL equivalent. See the
> [CLI command reference](CLI_COMMAND_REFERENCE.md#graph).

## Session commands

Session commands use the backslash prefix; the dot prefix is also accepted.

| Command | Description |
|---------|-------------|
| `\set <key> <value>` | Set a session parameter |
| `\show [key]` | Show all session settings or a specific one |
| `\reset [key]` | Reset one setting or all settings to defaults |
| `\use <collection>` | Set the active collection for the session |
| `\info` | Show database version, collection count, total points |
| `\bench <col> [n] [k]` | Quick benchmark (same as `.bench`) |

**Effect of `\use <collection>`:** sets the `collection` session setting. The
REPL verifies that the collection exists (vector, graph, or metadata) and
displays its type. `\use` records the active collection in the session state
(visible via `\show`), but VelesQL queries still require an explicit
`FROM <collection>` clause. The active collection does not automatically apply
to dot-commands or queries.

---

## Session settings

Session settings control REPL search behaviour. Set with `\set`, view with
`\show`, reset with `\reset`.

| Setting | Range / values | Default | Description |
|---------|---------------|---------|-------------|
| `mode` | `fast`, `balanced`, `accurate`, `perfect`, `adaptive` | `balanced` | Search quality preset (sets `ef_search` automatically) |
| `ef_search` | 16–4096 (or `auto` from mode) | auto | HNSW graph exploration factor |
| `timeout_ms` | >= 100 | 30000 | Query timeout in milliseconds. Also accepts the alias `timeout`. |
| `rerank` | `true`/`false`, `on`/`off`, `1`/`0`, `yes`/`no` | `true` | Reranking after quantized search |
| `max_results` | 1–10000 | 100 | Maximum results per query |
| `collection` | collection name | (none) | Active collection for `\use` |

```
velesdb> \set mode accurate
velesdb> \set ef_search 512
velesdb> \set timeout 5000
velesdb> \set rerank no
velesdb> \show
velesdb> \show mode
velesdb> \reset ef_search
velesdb> \reset
velesdb> \use documents
```

---

## VelesQL in the REPL

Any input not starting with `.` or `\` is executed as a VelesQL query through
the full pipeline (parse → validate → execute):

```
velesdb> SELECT * FROM docs WHERE category = 'tech' LIMIT 10;
velesdb> SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3] LIMIT 5;
velesdb> SELECT * FROM docs WHERE similarity(vector, [0.1, 0.2]) > 0.8 LIMIT 10;
velesdb> SELECT * FROM docs WHERE content MATCH 'rust programming' LIMIT 10;
velesdb> CREATE COLLECTION test (dimension = 4, metric = 'cosine');
velesdb> DROP COLLECTION test;
velesdb> SELECT EDGES FROM kg WHERE label = 'KNOWS';
velesdb> MATCH (a:Person)-[:KNOWS]->(b) RETURN a, b LIMIT 10;
velesdb> SELECT category, COUNT(*) FROM docs GROUP BY category HAVING COUNT(*) > 5;
velesdb> TRAIN QUANTIZER ON docs WITH (m = 8, k = 256);
```

**Supported statements:** SELECT, INSERT (with `$params`), UPSERT, UPDATE,
DELETE, CREATE/DROP COLLECTION, CREATE/DROP INDEX, TRUNCATE, FLUSH, ANALYZE,
SHOW COLLECTIONS, DESCRIBE COLLECTION, EXPLAIN, TRAIN QUANTIZER, INSERT EDGE,
DELETE EDGE, INSERT NODE, SELECT EDGES, MATCH (graph patterns),
UNION/INTERSECT/EXCEPT, JOIN.

**Supported WHERE clauses:** `=`, `!=`, `>`, `<`, `>=`, `<=`, `IN`, `NOT IN`,
`BETWEEN`, `IS NULL`, `IS NOT NULL`, `LIKE`, `ILIKE`, `NOT`, `AND`, `OR`,
`NEAR` (vector), `NEAR_FUSED` (multi-vector), `SPARSE_NEAR`, `MATCH` (BM25),
`similarity()` threshold.

**Supported modifiers:** `LIMIT`, `OFFSET`, `ORDER BY`, `GROUP BY`, `HAVING`,
`DISTINCT`, `WITH (mode, ef_search, timeout_ms, rerank, quantization)`,
`USING FUSION (rrf, rsf, weighted, maximum)`.

> **Limitation:** bind parameters (`$v`, `$query`) are not supported in the REPL
> because there is no mechanism to pass external values. Use literal vectors
> `[0.1, 0.2, ...]` for search queries. For INSERT with vectors, use the
> `velesdb data upsert` CLI command instead. Bind parameters work via the REST
> API (`POST /query` with `params`).

> **MATCH queries** require an active collection set via
> `\use <collection_name>`. The REPL tries graph collections first, then vector
> collections.

---

## Output formats

Two output formats, controlled by `.format` in the REPL or `--format` on CLI
subcommands:

- **table** (default) — UTF-8 formatted table with coloured headers. The `id`
  column is always first; remaining columns are sorted alphabetically. Missing
  or null values render as `-`.
- **json** — pretty-printed JSON array.

```
velesdb> .format json
velesdb> SELECT * FROM documents LIMIT 3;
```

```bash
velesdb query execute ./data "SELECT * FROM docs LIMIT 3" --format json
```

When a statement returns no rows, the REPL prints a kind-specific message
instead of an empty table: `No results.` (SELECT), `DDL statement executed
successfully.`, `DML statement executed successfully.`, `TRAIN statement
executed successfully.`, `Admin statement executed successfully.`, or
`No collections found.` (introspection).

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.3.0
