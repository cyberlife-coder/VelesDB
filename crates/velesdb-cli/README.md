# VelesDB CLI

[![Crates.io](https://img.shields.io/crates/v/velesdb-cli.svg)](https://crates.io/crates/velesdb-cli)
[![Build](https://img.shields.io/github/actions/workflow/status/cyberlife-coder/VelesDB/ci.yml?branch=main)](https://github.com/cyberlife-coder/VelesDB/actions)
[![License](https://img.shields.io/badge/license-MIT-green)](./LICENSE)

Interactive VelesQL REPL and CLI for VelesDB.

## Table of Contents

- [Installation](#installation)
- [Quick Start](#quick-start)
- [Getting Started](#getting-started)
- [REPL Mode](#repl-mode)
- [CLI Subcommands](#cli-subcommands)
- [VelesQL Examples](#velesql-examples) (Vector, Graph, Hybrid, Sparse, Temporal, Aggregations, Joins, Set Operations)
- [Session Settings](#session-settings)
- [Output Formats](#output-formats)
- [Shell Completions](#shell-completions)
- [Common Errors](#common-errors)
- [License](#license)

## Installation

### From crates.io

```bash
cargo install velesdb-cli
```

### From source

```bash
git clone https://github.com/cyberlife-coder/VelesDB
cd VelesDB
cargo install --path crates/velesdb-cli
```

### Linux .deb package

A `cargo-deb` configuration is included. Build the package with:

```bash
cargo deb -p velesdb-cli
sudo dpkg -i target/debian/velesdb-cli_*.deb
```

The binary is installed as `velesdb` in `/usr/bin/`.

## Quick Start

```bash
# Start the interactive REPL (default data directory: ./data)
velesdb repl

# Open a specific database
velesdb repl ./my_database

# Execute a one-shot query
velesdb query ./my_database "SELECT * FROM documents LIMIT 10"

# List all collections
velesdb list ./my_database
```

## Getting Started

End-to-end example: create a collection, insert vectors, search, and query the graph.

```bash
# 1. Create a vector collection (384-dimensional, cosine similarity)
velesdb create-vector-collection ./data docs --dimension 384 --metric cosine

# 2. Upsert a few points
velesdb upsert ./data docs --id 1 --vector '[0.1, 0.2, ..., 0.384]' \
  --payload '{"title": "Intro to VelesDB", "category": "tech"}'
velesdb upsert ./data docs --id 2 --vector '[0.3, 0.4, ..., 0.384]' \
  --payload '{"title": "Graph Databases 101", "category": "tech"}'

# 3. Search by vector similarity
velesdb query ./data "SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, ..., 0.384] LIMIT 5"

# 4. Create a graph collection and add edges
velesdb create-graph-collection ./data my_graph
velesdb graph add-edge ./data my_graph 1 100 200 "AUTHORED_BY"

# 5. Query the graph
velesdb graph neighbors ./data my_graph 100 --direction both

# 6. Enter the REPL for interactive exploration
velesdb repl ./data
```

## REPL Mode

Start the REPL with `velesdb repl [path]`. The REPL accepts dot-commands (`.help`), backslash-commands (`\set`), and raw VelesQL queries.

The REPL is single-line only -- each command or query must fit on one line. Multi-line input is not supported.

History is persisted across sessions in `~/.local/share/.velesdb_history` (Linux) or the equivalent platform data directory.

### General Commands

| Command | Aliases | Description |
|---------|---------|-------------|
| `.help` | `.h` | Show all available commands |
| `.quit` | `.exit`, `.q` | Exit the REPL |
| `.collections` | `.tables` | List all collections (vector, graph, metadata) |
| `.clear` | | Clear the terminal screen |
| `.timing on\|off` | | Toggle query execution time display (default: on). Also accepts `true`/`false`, `1`/`0`. |
| `.format table\|json` | | Set output format for query results |

### Collection Inspection

| Command | Aliases | Description |
|---------|---------|-------------|
| `.schema <name>` | | Show collection type, dimension, metric, and point count |
| `.describe <name>` | `.desc` | Detailed collection info (memory estimate, schema, storage mode) |
| `.stats <name>` | | Collection statistics (point count, dimension, memory) |
| `.count <name>` | | Show record/edge/item count |
| `.sample <name> [n]` | | Show first N records (default: 5). Works for Vector, Graph, and Metadata collections. |
| `.browse <name> [page]` | | Paginated record browsing (10 per page). Works for Vector, Graph, and Metadata collections. |
| `.nodes <name> [page]` | | Paginated node browsing for Graph collections (10 per page, includes payload). |

### Data Operations

| Command | Description |
|---------|-------------|
| `.export <name> [file]` | Export collection to JSON (default: `<name>.json`). Supports Vector and Metadata collections. |
| `.delete <name> <id> [id2...]` | Delete points by ID |
| `.flush <name>` | Flush collection data to disk |

### Query Analysis

| Command | Description |
|---------|-------------|
| `.explain <query>` | Show the execution plan for a VelesQL query (tree format) |
| `.analyze <name>` | Analyze collection: row count, deletion ratio, field stats, index stats |
| `.bench <name> [n] [k]` | Benchmark N random queries with top-k (default: 100 queries, k=10) |

### Index Management

| Command | Description |
|---------|-------------|
| `.indexes <name>` | List all indexes on a collection (type, cardinality, memory) |
| `.create-index <name> <field> [--type secondary\|property\|range]` | Create an index (default: secondary) |
| `.drop-index <name> <label> <property>` | Drop an index by label and property |

### Advanced Search

| Command | Description |
|---------|-------------|
| `.sparse-search <col> <index> <json> [k]` | Sparse vector search. JSON format: `[[idx, weight], ...]`. Default k=10. |
| `.hybrid-sparse <col> <dense> <sparse> [k] [--strategy rrf\|average\|max] [--index <name>]` | Dense+sparse hybrid search with fusion. Default k=10, strategy=rrf. |
| `.guardrails` | Display current query guard-rails (timeout, memory limit, rate limit, circuit breaker) |
| `.agent [cmd]` | Agent memory commands (preview -- not yet fully implemented in CLI) |

**`.sparse-search` example:**

```
velesdb> .sparse-search my_col sparse_idx [[42,0.8],[137,0.6],[891,0.3]] 10
```

**`.hybrid-sparse` example:**

Dense vector is a JSON array `[0.1, 0.2, ...]`, sparse vector is `[[index, weight], ...]`:

```
velesdb> .hybrid-sparse docs [0.1,0.2,0.3,0.4] [[0,1.5],[3,0.8]] 10 --strategy rrf
```

Valid strategies: `rrf`, `average`, `max` (also accepts `maximum`).

### Graph Commands (REPL)

All graph REPL commands operate on graph collections via `.graph <subcommand>`:

| Command | Description |
|---------|-------------|
| `.graph add-edge <col> <id> <src> <tgt> <label>` | Add a directed edge |
| `.graph edges <col> [--label <label>]` | List edges, optionally filtered by label |
| `.graph degree <col> <node_id>` | Show in-degree, out-degree, and total degree |
| `.graph traverse <col> <source> [--algo bfs\|dfs] [--depth N] [--limit N]` | BFS/DFS traversal from a source node |
| `.graph neighbors <col> <node_id> [--direction in\|out\|both]` | List neighbors of a node (default: out) |
| `.nodes <col> [page]` | Paginated browsing of all nodes referenced in edges (with payload if stored) |

> **Note:** `store-payload` and `get-payload` are available as **CLI subcommands only** (`velesdb graph store-payload`, `velesdb graph get-payload`), not as REPL dot-commands.

> **Naming note:** The REPL uses `.graph edges` while the CLI subcommand uses `graph get-edges`. Both do the same thing.

### Session Commands

Session commands use the backslash prefix (dot prefix also accepted):

| Command | Description |
|---------|-------------|
| `\set <key> <value>` | Set a session parameter |
| `\show [key]` | Show all session settings or a specific one |
| `\reset [key]` | Reset one setting or all settings to defaults |
| `\use <collection>` | Set the active collection for the session |
| `\info` | Show database version, collection count, total points |
| `\bench <col> [n] [k]` | Quick benchmark (same as `.bench`) |

**Effect of `\use <collection>`:** Sets the `collection` session setting to the named collection. The REPL verifies that the collection exists (vector, graph, or metadata) and displays its type. Currently, `\use` records the active collection in the session state (visible via `\show`), but VelesQL queries still require an explicit `FROM <collection>` clause. The active collection does not automatically apply to dot-commands or queries.

### VelesQL Queries

Any input not starting with `.` or `\` is executed as a VelesQL query:

```
velesdb> SELECT * FROM documents WHERE category = 'tech' LIMIT 10;
velesdb> SELECT * FROM docs WHERE vector NEAR [0.1, 0.2, 0.3] LIMIT 5;
```

## CLI Subcommands

All subcommands operate offline against the database directory -- no running server required.

### Collections

```bash
# Create a vector collection
velesdb create-vector-collection ./data my_vectors \
  --dimension 384 \
  --metric cosine \
  --storage full

# Create a graph collection
velesdb create-graph-collection ./data my_graph --schemaless true

# Create a metadata-only collection (no vectors, no graph -- structured payloads only)
velesdb create-metadata-collection ./data my_metadata

# List all collections
velesdb list ./data
velesdb list ./data --format json

# Show collection details
velesdb show ./data my_vectors
velesdb show ./data my_vectors --samples 5 --format json

# Delete a collection (interactive confirmation)
velesdb delete-collection ./data my_vectors
velesdb delete-collection ./data my_vectors --force

# Database overview
velesdb info ./data
```

**`create-vector-collection` flags:**

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `-d, --dimension` | integer | (required) | Vector dimension |
| `-m, --metric` | `cosine`, `euclidean`, `dot` (aliases: `dotproduct`, `inner`, `ip`), `hamming`, `jaccard` | `cosine` | Distance metric |
| `-s, --storage` | `full` (`f32`), `sq8` (`int8`), `binary` (`bit`), `pq`, `rabitq` | `full` | Storage/quantization mode |

**`create-metadata-collection`:** Creates a collection that stores only structured JSON payloads -- no vectors, no graph edges. Useful for reference tables, configuration, or any metadata that does not need similarity search. There are no additional flags beyond the database path and collection name.

### Points

```bash
# Upsert a point with vector and payload
velesdb upsert ./data my_vectors \
  --id 1 \
  --vector '[0.1, 0.2, 0.3]' \
  --payload '{"title": "Hello World"}'

# Upsert with payload only (no --vector flag)
# The vector defaults to an empty array. This will fail if the collection
# expects a specific dimension -- the collection validates vector dimensions.
velesdb upsert ./data my_vectors \
  --id 2 \
  --payload '{"title": "No vector"}'

# Get a point by ID (default output format: json)
velesdb get ./data my_vectors 42
velesdb get ./data my_vectors 42 --format table

# Delete points
velesdb delete-points ./data my_vectors 1 2 3
```

The `upsert` subcommand operates on **vector collections only**. The `--vector` flag is optional (an empty vector is used when omitted), but the collection will reject vectors whose dimension does not match the configured dimension -- this produces a dimension mismatch error. The `--id` flag is required.

### Import / Export

```bash
# Import from JSONL
velesdb import data.jsonl \
  --database ./data \
  --collection documents \
  --dimension 768 \
  --metric cosine \
  --batch-size 1000

# Import from CSV (custom column names)
velesdb import embeddings.csv \
  --database ./data \
  --collection docs \
  --id-column doc_id \
  --vector-column embedding

# Export to JSON (vector collections only)
velesdb export ./data documents --output documents.json

# Export metadata only (no vectors)
velesdb export ./data documents --output meta.json --no-include-vectors
```

> **Note on `--include-vectors`:** This is a Clap boolean flag that defaults to `true`. To disable it, use `--no-include-vectors` (Clap's automatic negation syntax). Writing `--include-vectors false` does **not** work as expected with Clap.

**JSONL format for `import`:**

Each line must be a valid JSON object with the following fields:

```jsonl
{"id": 1, "vector": [0.1, 0.2, 0.3, 0.4], "payload": {"title": "Doc A", "category": "tech"}}
{"id": 2, "vector": [0.5, 0.6, 0.7, 0.8], "payload": {"title": "Doc B"}}
{"id": 3, "vector": [0.9, 0.0, 0.1, 0.2]}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | `u64` | yes | Unique point identifier |
| `vector` | `[f32]` | yes | Dense vector (must match the collection dimension) |
| `payload` | JSON object | no | Arbitrary JSON metadata |

Lines with mismatched vector dimensions are counted as errors and skipped.

**`import` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `-d, --database` | `./data` | Database directory |
| `-c, --collection` | (required) | Target collection name |
| `--dimension` | auto-detected | Vector dimension (detected from first record if omitted) |
| `--metric` | `cosine` | Distance metric (`cosine`, `euclidean`, `dot`/`ip`, `hamming`, `jaccard`) |
| `--storage-mode` | `full` | Storage mode (`full`/`f32`, `sq8`/`int8`, `binary`/`bit`, `pq`, `rabitq`) |
| `--id-column` | `id` | ID column name (CSV only) |
| `--vector-column` | `vector` | Vector column name (CSV only) |
| `--batch-size` | `1000` | Insertion batch size |
| `--progress` | `true` | Show progress bar |

**`export` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output` | `<collection>.json` | Output file path |
| `--include-vectors` / `--no-include-vectors` | `true` | Include vector data in export |

Export operates on **vector collections only**. Attempting to export a graph or metadata collection produces a "not found" error.

### Search

```bash
# Execute a single VelesQL query
velesdb query ./data "SELECT * FROM documents LIMIT 10"
velesdb query ./data "SELECT * FROM docs WHERE category = 'tech' LIMIT 5" --format json

# Multi-query fusion search
velesdb multi-search ./data my_vectors \
  '[[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]' \
  -k 10 \
  --strategy rrf \
  --rrf-k 60

# Explain a query plan (default format: tree)
velesdb explain ./data "SELECT * FROM docs WHERE vector NEAR [0.1, 0.2] LIMIT 5"
velesdb explain ./data "SELECT * FROM docs LIMIT 10" --format json
```

**`multi-search` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `-k, --top-k` | `10` | Number of results to return |
| `-s, --strategy` | `rrf` | Fusion strategy: `average`, `maximum`, `rrf`, `weighted` |
| `--rrf-k` | `60` | RRF k parameter (only used with `rrf` strategy) |
| `-f, --format` | `table` | Output format (`table`, `json`) |

**`multi-search` strategies:** `average` (also `avg`), `maximum` (also `max`), `rrf` (default), `weighted`

The `weighted` strategy uses fixed weights (`avg_weight=0.5`, `max_weight=0.3`, `hit_weight=0.2`) that are not configurable via the CLI.

**`explain` formats:**

The default format is `tree`, which renders a human-readable execution plan tree. Use `--format json` for machine-readable output.

Example tree output:

```
velesdb> .explain SELECT * FROM docs WHERE category = 'tech' LIMIT 10
Plan:
  Scan: docs
    Filter: category = 'tech'
    Limit: 10
  Estimated cost: 0.150 ms
```

### Analyze

```bash
# Collection statistics (point count, deletion ratio, index stats, column stats)
velesdb analyze ./data my_vectors
velesdb analyze ./data my_vectors --format json
```

### Graph

All graph CLI subcommands require a database path and graph collection name:

```bash
# Add an edge
velesdb graph add-edge ./data my_graph 1 100 200 "AUTHORED_BY"

# List edges
velesdb graph get-edges ./data my_graph
velesdb graph get-edges ./data my_graph --label "AUTHORED_BY" --format json

# Node degree
velesdb graph degree ./data my_graph 100

# Traverse (BFS/DFS)
velesdb graph traverse ./data my_graph 100 \
  --algorithm bfs \
  --max-depth 3 \
  --limit 50 \
  --rel-types "AUTHORED_BY,CITES"

# Get neighbors
velesdb graph neighbors ./data my_graph 100 --direction both --format json

# Store payload on a graph node
velesdb graph store-payload ./data my_graph 100 '{"name": "Alice", "role": "author"}'

# Retrieve node payload
velesdb graph get-payload ./data my_graph 100

# List all nodes (paginated, 20 per page)
velesdb graph nodes ./data my_graph
velesdb graph nodes ./data my_graph --page 2
velesdb graph nodes ./data my_graph --format json
```

**`traverse` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `--algorithm` | `bfs` | `bfs` or `dfs` |
| `-d, --max-depth` | `3` | Maximum traversal depth |
| `-l, --limit` | `100` | Maximum number of results |
| `-r, --rel-types` | (all) | Comma-separated relationship type filter |
| `-f, --format` | `table` | Output format (`table`, `json`) |

### Index Management

```bash
# Create a secondary index
velesdb index create ./data my_vectors category

# Create a property index
velesdb index create ./data my_vectors name --index-type property --label Person

# Create a range index
velesdb index create ./data my_vectors price --index-type range --label Product

# List indexes
velesdb index list ./data my_vectors
velesdb index list ./data my_vectors --format json

# Drop an index
velesdb index drop ./data my_vectors Person name
```

### SIMD Diagnostics

```bash
# Show SIMD dispatch configuration
velesdb simd info

# Run SIMD benchmarks (redirects to cargo bench)
velesdb simd benchmark
```

### License Management

```bash
# Show current license status
velesdb license show

# Activate a license
velesdb license activate <license_key>

# Verify a license (without activating)
velesdb license verify <license_key> --public-key <base64_public_key>
```

Both `license show` and `license activate` read the public key from the `VELESDB_LICENSE_PUBLIC_KEY` environment variable. If the variable is not set, a development fallback key is used with a warning. Set it with:

```bash
export VELESDB_LICENSE_PUBLIC_KEY=<base64_encoded_public_key>
```

### Shell Completions

```bash
# Generate completions for your shell
velesdb completions bash       > /etc/bash_completion.d/velesdb
velesdb completions zsh        > ~/.zfunc/_velesdb
velesdb completions fish       > ~/.config/fish/completions/velesdb.fish
velesdb completions powershell > velesdb.ps1
velesdb completions elvish     > velesdb.elv
```

## VelesQL Examples

VelesQL is a SQL-like query language with vector and graph extensions. These examples work directly in the REPL.

### Vector Search

```sql
-- Basic vector similarity search
SELECT * FROM documents
WHERE vector NEAR [0.15, 0.25, 0.35, 0.45]
LIMIT 10;

-- Vector search with metadata filter
SELECT * FROM documents
WHERE vector NEAR [0.1, 0.2, 0.3, 0.4]
AND category = 'tech'
AND views > 100
LIMIT 5;
```

> The distance metric is defined at collection creation time and applies to all searches on that collection. All five metrics are supported: Cosine, Euclidean, DotProduct, Hamming, Jaccard.

### Multi-Vector Fusion (NEAR_FUSED)

```sql
-- RRF fusion with multiple query vectors
SELECT * FROM documents
WHERE vector NEAR_FUSED [$v1, $v2, $v3] USING FUSION 'rrf' (k = 60)
LIMIT 10;

-- Weighted fusion
SELECT * FROM documents
WHERE vector NEAR_FUSED [$query1, $query2] USING FUSION 'weighted'
LIMIT 10;

-- Maximum fusion
SELECT * FROM documents
WHERE vector NEAR_FUSED [$v1, $v2] USING FUSION 'maximum'
LIMIT 10;

-- Default (RRF) -- USING FUSION clause is optional
SELECT * FROM documents
WHERE vector NEAR_FUSED [$v1, $v2]
LIMIT 10;
```

### Hybrid Search (USING FUSION)

The `USING FUSION` clause at query level combines results from multiple search strategies (vector + text, dense + sparse):

```sql
-- Dense vector + BM25 full-text combined with RRF
SELECT * FROM documents
WHERE vector NEAR $v AND content MATCH 'rust programming'
USING FUSION(strategy = 'rrf', k = 60)
LIMIT 10;

-- Dense + sparse vector fusion with RSF
SELECT * FROM documents
WHERE vector NEAR $dense AND vector SPARSE_NEAR $sparse
USING FUSION(strategy = 'rsf', dense_w = 0.7, sparse_w = 0.3)
LIMIT 10;

-- Weighted fusion
SELECT * FROM docs
WHERE vector NEAR $v
USING FUSION(strategy = 'weighted', vector_weight = 0.7, graph_weight = 0.3)
LIMIT 10;

-- Maximum fusion (take best score)
SELECT * FROM docs
WHERE vector NEAR $v
USING FUSION(strategy = 'maximum')
LIMIT 10;

-- Default USING FUSION (defaults to RRF)
SELECT * FROM docs
WHERE vector NEAR $v
USING FUSION
LIMIT 10;
```

### Graph MATCH Queries

```sql
-- Find authors of documents similar to a query
MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
WHERE similarity(doc.embedding, $question) > 0.8
RETURN author.name, doc.title
ORDER BY similarity() DESC
LIMIT 5;

-- Multi-hop traversal with depth range
MATCH (user:User)-[:FOLLOWS*1..3]->(target:User)
WHERE user.name = 'Alice'
RETURN target.name, target.bio
LIMIT 20;

-- Undirected relationship (both directions)
MATCH (a:Person)-[:KNOWS]-(b:Person)
WHERE a.city = 'Paris'
RETURN a.name, b.name
LIMIT 10;

-- Incoming relationships
MATCH (doc:Document)<-[:AUTHORED_BY]-(author:Person)
RETURN doc.title, author.name
LIMIT 10;

-- Node property filtering in pattern
MATCH (doc:Document {status: 'published'})-[:HAS_TAG]->(tag:Tag)
RETURN doc.title, tag.name
LIMIT 20;

-- Combined: graph + vector + metadata in WHERE
SELECT * FROM articles
WHERE category = 'tech' AND MATCH (d:Doc)-[:HAS_TAG]->(tag)
LIMIT 10;
```

### Metadata-Only Collections

Metadata collections store structured data without vectors. They support full VelesQL `SELECT` queries:

```sql
-- Browse all items in a metadata collection
SELECT * FROM my_metadata LIMIT 10;

-- Filter by field
SELECT * FROM my_metadata WHERE status = 'active' LIMIT 20;

-- Count items
SELECT * FROM my_metadata WHERE price > 100 LIMIT 100;
```

> **Tip:** Use `.sample my_metadata`, `.browse my_metadata`, and `.export my_metadata` in the REPL —
> all three commands work for Metadata collections (no "vector" column is shown).

### Metadata Filters

```sql
-- Equality, comparison, pattern matching
SELECT * FROM docs WHERE category = 'tech' LIMIT 10;
SELECT * FROM docs WHERE price >= 50 AND price <= 200 LIMIT 10;
SELECT * FROM docs WHERE title LIKE '%rust%' LIMIT 10;

-- Case-insensitive pattern matching
SELECT * FROM docs WHERE title ILIKE '%Rust%' LIMIT 10;

-- IN, BETWEEN, NULL checks
SELECT * FROM docs WHERE status IN ('published', 'featured') LIMIT 10;
SELECT * FROM docs WHERE score BETWEEN 0.5 AND 1.0 LIMIT 10;
SELECT * FROM docs WHERE author IS NOT NULL LIMIT 10;

-- NOT and OR operators
SELECT * FROM docs WHERE NOT category = 'draft' LIMIT 10;
SELECT * FROM docs WHERE category = 'tech' OR category = 'science' LIMIT 10;

-- Nested field access (dot notation)
SELECT metadata.source, profile.type FROM docs WHERE metadata.lang = 'en' LIMIT 10;

-- Full-text search (BM25)
SELECT * FROM docs WHERE content MATCH 'rust programming' LIMIT 10;
```

### Similarity Threshold

```sql
-- Return all documents above a similarity threshold (not just top-K)
SELECT * FROM docs
WHERE similarity(vector, $query) > 0.8
LIMIT 20;

-- Combine similarity threshold with metadata filters
SELECT * FROM docs
WHERE similarity(embedding, $ref) >= 0.9 AND category = 'tech'
LIMIT 10;
```

### Sparse Vector Search

```sql
-- Sparse vector similarity search
SELECT * FROM docs
WHERE vector SPARSE_NEAR $sparse_vector
LIMIT 10;

-- Sparse search with inline literal
SELECT * FROM docs
WHERE vector SPARSE_NEAR {12: 0.8, 45: 0.3, 891: 0.1}
LIMIT 10;

-- Sparse search on a named index
SELECT * FROM docs
WHERE vector SPARSE_NEAR $sv USING 'my_sparse_index'
LIMIT 10;
```

### Temporal Queries

```sql
-- Filter by current time
SELECT * FROM events WHERE timestamp > NOW() LIMIT 10;

-- Last 7 days
SELECT * FROM logs WHERE created_at > NOW() - INTERVAL '7 days' LIMIT 50;

-- Last hour
SELECT * FROM alerts WHERE fired_at > NOW() - INTERVAL '1 hour' LIMIT 20;

-- Next week (scheduling)
SELECT * FROM tasks WHERE due_date < NOW() + INTERVAL '7 days' LIMIT 20;

-- Shorthand units: s, m, h, d, w, month
SELECT * FROM metrics WHERE ts > NOW() - INTERVAL '30 min' LIMIT 100;
```

### DISTINCT

```sql
-- Deduplicate results
SELECT DISTINCT category FROM documents LIMIT 50;

SELECT DISTINCT status, priority FROM tasks LIMIT 20;
```

### Aggregations

```sql
SELECT category, COUNT(*) as cnt
FROM documents
GROUP BY category
HAVING cnt > 5
ORDER BY cnt DESC
LIMIT 10;

-- Multiple aggregates
SELECT category, COUNT(*) as cnt, AVG(price) as avg_price, MIN(price), MAX(price)
FROM products
GROUP BY category
LIMIT 20;

-- SUM aggregate
SELECT region, SUM(quantity) as total
FROM orders
GROUP BY region
ORDER BY total DESC
LIMIT 10;

-- GROUP BY nested fields
SELECT metadata.source, COUNT(*) as cnt
FROM documents
GROUP BY metadata.source
LIMIT 20;
```

### ORDER BY

```sql
-- Multiple sort keys
SELECT * FROM docs ORDER BY category ASC, created_at DESC LIMIT 20;

-- Order by similarity score
SELECT * FROM docs
WHERE vector NEAR $v
ORDER BY similarity(vector, $v) DESC
LIMIT 10;
```

### OFFSET (Pagination)

```sql
-- Skip first 20 results, return next 10
SELECT * FROM docs LIMIT 10 OFFSET 20;
```

### Set Operations

```sql
-- UNION: combine results from two queries
SELECT id, title FROM news WHERE category = 'tech'
UNION
SELECT id, title FROM blog WHERE category = 'tech';

-- UNION ALL: include duplicates
SELECT id FROM collection_a
UNION ALL
SELECT id FROM collection_b;

-- INTERSECT: rows in both queries
SELECT id FROM favorites INTERSECT SELECT id FROM published;

-- EXCEPT: rows in first but not second
SELECT id FROM all_items EXCEPT SELECT id FROM archived;
```

### JOIN

```sql
-- INNER JOIN (default)
SELECT o.id, c.name
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.id
LIMIT 20;

-- LEFT JOIN
SELECT d.title, a.name
FROM documents AS d
LEFT JOIN authors AS a ON d.author_id = a.id
LIMIT 20;

-- JOIN with vector search
SELECT o.id, c.name
FROM orders AS o
JOIN customers AS c ON o.customer_id = c.id
WHERE similarity(o.embedding, $q) > 0.7
LIMIT 20;
```

> **Note:** `LEFT JOIN` and `RIGHT JOIN` are parsed but raise runtime errors. `INNER JOIN` is fully supported.

### Subqueries (parsed, not yet executable)

> **Note:** Subqueries are recognized by the VelesQL parser but raise runtime errors during execution. This syntax is reserved for future support.

```sql
-- IN subquery (parsed, execution not yet supported)
SELECT * FROM docs WHERE id IN (SELECT doc_id FROM comments) LIMIT 10;

-- Scalar subquery comparison (parsed, execution not yet supported)
SELECT * FROM products WHERE price > (SELECT AVG(price) FROM products) LIMIT 20;
```

### EXPLAIN

```sql
-- Show the query execution plan
EXPLAIN SELECT * FROM documents
WHERE vector NEAR [0.1, 0.2, 0.3, 0.4]
AND category = 'tech'
LIMIT 10;
```

### TRAIN QUANTIZER

```sql
-- Train a product quantizer on a collection
TRAIN QUANTIZER ON documents WITH (m = 8, k = 256);

-- With oversampling and force retrain
TRAIN QUANTIZER ON large_docs WITH (m = 16, k = 256, oversampling = 4, force = true);
```

| Parameter | Description | Typical Values |
|-----------|-------------|----------------|
| `m` | Number of sub-spaces to divide the vector into. Higher = better recall, slower training. | 4, 8, 16, 32 |
| `k` | Number of centroids per sub-space. Almost always 256 (one byte per sub-quantizer). | 256 |
| `oversampling` | Oversampling factor for training data | 2, 4 |
| `force` | Force retraining even if quantizer exists | `true`, `false` |

### WITH Clause (Per-Query Options)

```sql
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (mode = 'accurate');

SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (ef_search = 512, timeout_ms = 5000, rerank = true);

-- Quantization hints for dual-precision search
SELECT * FROM docs WHERE vector NEAR $v LIMIT 10
WITH (quantization = 'dual', oversampling = 4);
```

| Option | Type | Description |
|--------|------|-------------|
| `mode` | string | `fast`, `balanced`, `accurate`, `perfect`, `adaptive` |
| `ef_search` | integer | HNSW ef_search (16--4096) |
| `timeout_ms` | integer | Query timeout in milliseconds |
| `rerank` | boolean | Enable reranking after quantized search |
| `quantization` | string | Quantization precision: `f32`, `int8`, `dual`, `auto` |
| `oversampling` | integer | Oversampling ratio for dual-precision mode (>= 1) |

### Escaped Identifiers

Use backticks or double-quotes to use reserved words as column names:

```sql
-- Backtick style
SELECT `select`, `from`, `order` FROM docs LIMIT 10;

-- Double-quote style (SQL standard)
SELECT "select", "from", "order" FROM docs LIMIT 10;
```

## Session Settings

Session settings control REPL search behavior. Set with `\set`, view with `\show`, reset with `\reset`.

| Setting | Range / Values | Default | Description |
|---------|---------------|---------|-------------|
| `mode` | `fast`, `balanced`, `accurate`, `perfect`, `adaptive` | `balanced` | Search quality preset (sets `ef_search` automatically) |
| `ef_search` | 16--4096 (or `auto` from mode) | auto | HNSW graph exploration factor |
| `timeout_ms` | >= 100 | 30000 | Query timeout in milliseconds. Also accepts the alias `timeout`. |
| `rerank` | `true`/`false`, `on`/`off`, `1`/`0`, `yes`/`no` | `true` | Reranking after quantized search |
| `max_results` | 1--10000 | 100 | Maximum results per query |
| `collection` | collection name | (none) | Active collection for `\use` |

Examples:

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

## Output Formats

Two output formats are available, controlled by `.format` in the REPL or `--format` on CLI subcommands:

- **table** (default) -- UTF-8 formatted table with colored headers. The `id` column is always first.
- **json** -- Pretty-printed JSON array.

```
velesdb> .format json
velesdb> SELECT * FROM documents LIMIT 3;
```

```bash
velesdb query ./data "SELECT * FROM docs LIMIT 3" --format json
```

## Shell Completions

Generate tab-completion scripts for your shell:

```bash
# Bash
velesdb completions bash > /etc/bash_completion.d/velesdb

# Zsh (add ~/.zfunc to fpath in .zshrc)
velesdb completions zsh > ~/.zfunc/_velesdb

# Fish
velesdb completions fish > ~/.config/fish/completions/velesdb.fish

# PowerShell
velesdb completions powershell | Out-String | Invoke-Expression

# Elvish
velesdb completions elvish > ~/.config/elvish/lib/velesdb.elv
```

## Common Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `Collection 'X' not found` | The collection does not exist, or you used a command that expects a specific type (e.g., `upsert` requires a vector collection but 'X' is a graph collection). | Check the name with `.collections`. Use the correct command for the collection type. |
| `Vector collection 'X' not found` | A vector-specific command (`upsert`, `export`, `delete-points`, `get`) was used on a graph or metadata collection. | Use the correct collection type or create a vector collection. |
| `Graph collection 'X' not found` | A graph command was used but the collection is not a graph collection. | Verify the collection type with `.schema X`. |
| Dimension mismatch on upsert/import | The vector length does not match the collection's configured dimension. | Ensure all vectors have the correct number of elements. During import, mismatched lines are skipped and counted in `errors`. |
| `Failed to open database` | The database path is incorrect, the directory does not exist, or insufficient permissions. | Verify the path exists and is writable. |
| `null` from `get` | The requested point ID does not exist in the collection. | The `get` command prints `null` (JSON format) or shows "Point with ID X not found" (table format). |
| `Parse error: ...` | Invalid VelesQL syntax. | Check the query syntax against the VelesQL spec. |
| `Empty file` | The import file contains no records. | Verify the file is not empty and has the correct format (.jsonl or .csv). |

## License

MIT License

See [LICENSE](./LICENSE) for details.
