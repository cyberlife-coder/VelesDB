# `velesdb` command reference

Complete reference for the `velesdb` binary shipped by the
[`velesdb-cli`](../../crates/velesdb-cli/README.md) crate.

All subcommands operate **offline** against a database directory — no running
server is required. Every command below was executed against `velesdb 4.0.0`.

Related: [REPL reference](CLI_REPL_REFERENCE.md) ·
[VelesQL cookbook](CLI_VELESQL_COOKBOOK.md) ·
[VelesQL spec](../VELESQL_SPEC.md)

---

## Table of contents

- [Global options](#global-options)
- [Collections](#collections)
- [Points](#points)
- [Import / export](#import--export)
- [Query](#query)
- [Analyze](#analyze)
- [Graph](#graph)
- [Index management](#index-management)
- [SIMD diagnostics](#simd-diagnostics)
- [License management](#license-management)
- [Shell completions](#shell-completions)
- [Building the Debian package](#building-the-debian-package)
- [Error reference](#error-reference)

---

## Global options

| Option | Environment variable | Effect |
|---|---|---|
| `--config <FILE>` | `VELESDB_CONFIG` | Path to a VelesDB TOML config file (search/HNSW/storage/limits/WAL batching). Applies to every command that opens a database, including the REPL. An invalid or missing path fails fast — it never silently falls back to defaults. |

```bash
velesdb --config ./velesdb.toml collection list ./data
```

Two more environment variables affect the binary:

| Variable | Effect |
|---|---|
| `VELESDB_NO_UPDATE_CHECK=1` | Disables the non-blocking background update check (built with the default `update-check` feature). Can also be disabled with `[update_check] enabled = false` in the config file. |
| `VELESDB_LICENSE_PUBLIC_KEY` | Base64 Ed25519 public key used by `license show` and `license activate`. Unset means a development fallback key is used, with a warning. |

---

## Collections

```bash
# Create a vector collection
velesdb collection create ./data my_vectors \
  --dimension 384 \
  --metric cosine \
  --storage full

# Create a graph collection (schemaless)
velesdb collection create-graph ./data my_graph

# Create a metadata-only collection (no vectors, no graph — structured payloads only)
velesdb collection create-metadata ./data my_metadata

# List all collections
velesdb collection list ./data
velesdb collection list ./data --format json

# Show collection details
velesdb collection show ./data my_vectors
velesdb collection show ./data my_vectors --samples 5 --format json

# Delete a collection (interactive confirmation, type `yes`)
velesdb collection delete ./data my_vectors
velesdb collection delete ./data my_vectors --force

# Database overview
velesdb info ./data
```

**`collection create` flags:**

| Flag | Values | Default | Description |
|------|--------|---------|-------------|
| `-d, --dimension` | integer | (required) | Vector dimension |
| `-m, --metric` | `cosine`, `euclidean`, `dot`, `hamming`, `jaccard` | `cosine` | Distance metric |
| `-s, --storage` | `full`, `sq8`, `binary`, `pq`, `rabitq` | `full` | Storage/quantization mode |

**`collection create-metadata`:** creates a collection that stores only
structured JSON payloads — no vectors, no graph edges. Useful for reference
tables, configuration, or any metadata that does not need similarity search.
There are no flags beyond the database path and the collection name.

---

## Points

```bash
# Upsert a point with vector and payload
velesdb data upsert ./data my_vectors \
  --id 1 \
  --vector '[0.1, 0.2, 0.3]' \
  --payload '{"title": "Hello World"}'

# Upsert with payload only (no --vector flag)
# The vector defaults to an empty array. This fails when the collection
# expects a specific dimension — the collection validates vector dimensions.
velesdb data upsert ./data my_vectors \
  --id 2 \
  --payload '{"title": "No vector"}'

# Get a point by ID (default output format: json)
velesdb data get ./data my_vectors 42
velesdb data get ./data my_vectors 42 --format table

# Delete points
velesdb data delete ./data my_vectors 1 2 3
```

`data upsert` operates on **vector collections only**. The `--vector` flag is
optional (an empty vector is used when omitted), but the collection rejects
vectors whose dimension does not match the configured dimension — that produces
`[VELES-004] Vector dimension mismatch: expected N, got M`. The `--id` flag is
required.

### Cursor pagination — `data scroll`

```bash
velesdb data scroll ./data my_vectors --batch-size 20
velesdb data scroll ./data my_vectors --batch-size 20 --cursor 20
velesdb data scroll ./data my_vectors --format table
```

| Flag | Default | Description |
|------|---------|-------------|
| `-b, --batch-size` | `20` | Points per page |
| `--cursor` | (first page) | Point ID to resume after |
| `-f, --format` | `json` | `table` or `json` |

JSON output carries the cursor for the next call:

```json
{
  "nextCursor": 1,
  "points": [
    {
      "id": 1,
      "payload": { "category": "tech", "title": "Rust in action" },
      "vector": [1.0, 0.0, 0.0, 0.0]
    }
  ]
}
```

### Streaming ingest — `data stream-insert`

Reads one JSON object per line from **stdin** and micro-batches the upserts:

```bash
printf '{"id":10,"vector":[0.1,0.2,0.3,0.4],"payload":{"title":"streamed"}}\n' \
  | velesdb data stream-insert ./data my_vectors --batch-size 100
```

```
✅ Stream insert complete: 1 inserted, 0 errors
```

| Flag | Default | Description |
|------|---------|-------------|
| `-b, --batch-size` | `100` | Points buffered before each upsert |

---

## Import / export

```bash
# Import from JSONL
velesdb data import data.jsonl \
  --database ./data \
  --collection documents \
  --dimension 768 \
  --metric cosine \
  --batch-size 1000

# Import from CSV (custom column names)
velesdb data import embeddings.csv \
  --database ./data \
  --collection docs \
  --id-column doc_id \
  --vector-column embedding

# Import from a VRB1 binary file (.bin / .vrb1) — zero-copy bulk path
velesdb data import vectors.bin \
  --database ./data \
  --collection docs

# Export to JSON (vector collections only)
velesdb data export ./data documents --output documents.json

# Export payloads only (omit vectors)
velesdb data export ./data documents --output meta.json --include-vectors false
```

> **Note on `--include-vectors`:** vectors and payloads are included by default.
> Pass `--include-vectors false` to export payloads only (the bare
> `--include-vectors` form still means "include").

### JSONL format for `data import`

Each line must be a valid JSON object:

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

### VRB1 binary format for `data import` (`.bin` / `.vrb1`, since 2026-06-14)

A `.bin` / `.vrb1` file uses the VRB1 little-endian wire format: a 16-byte
header (`b"VRB1"` magic, `u32` count, `u32` dimension, `u8` id width = 8,
3 reserved zero bytes) followed by tightly-packed `u64` ids and row-major `f32`
vectors. It feeds the zero-copy bulk path and carries **no payloads** — use
`.jsonl` / `.csv` when payloads are needed. The declared dimension sizes a
freshly created collection, so `--dimension` is not required for `.bin` imports.

Any other extension is rejected with
`Unsupported file format: <ext>. Use .csv, .jsonl, or .bin (VRB1)`.

**`data import` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `-d, --database` | `./data` | Database directory |
| `-c, --collection` | (required) | Target collection name |
| `--dimension` | auto-detected | Vector dimension (detected from the first record if omitted) |
| `--metric` | `cosine` | Distance metric (`cosine`, `euclidean`, `dot`, `hamming`, `jaccard`) |
| `--storage-mode` | `full` | Storage mode (`full`, `sq8`, `binary`, `pq`, `rabitq`) |
| `--id-column` | `id` | ID column name (CSV only) |
| `--vector-column` | `vector` | Vector column name (CSV only) |
| `--batch-size` | `1000` | Insertion batch size |
| `--progress [true\|false]` | `true` | Show progress bar (`--progress false` to disable) |

**`data export` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `-o, --output` | `<collection>.json` | Output file path |
| `--include-vectors [true\|false]` | `true` | Include vector data (`--include-vectors false` to omit) |

Export operates on **vector collections only**. Exporting a graph or metadata
collection fails with
`Vector collection 'X' not found. Export requires a vector collection.`

---

## Query

```bash
# Execute a single VelesQL query
velesdb query execute ./data "SELECT * FROM documents LIMIT 10"
velesdb query execute ./data "SELECT * FROM docs WHERE category = 'tech' LIMIT 5" --format json

# Target a collection for a query with no FROM clause (e.g. a bare MATCH),
# mirroring the REST /query `collection` field
velesdb query execute ./data "MATCH (a)-[:KNOWS]->(b) RETURN a, b LIMIT 5" \
  --collection my_graph

# Multi-query fusion search (one fused result list)
velesdb query search ./data my_vectors \
  '[[0.1, 0.2, 0.3], [0.4, 0.5, 0.6]]' \
  -k 10 \
  --strategy rrf \
  --rrf-k 60

# Batch search (independent result list per query vector, run in parallel)
velesdb query batch-search ./data my_vectors \
  '[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]' -k 1

# Explain a query plan (default format: tree)
velesdb query explain ./data "SELECT * FROM docs WHERE vector NEAR [0.1, 0.2] LIMIT 5"
velesdb query explain ./data "SELECT * FROM docs LIMIT 10" --format json
```

**`query search` flags:**

| Flag | Default | Description |
|------|---------|-------------|
| `-k, --top-k` | `10` | Number of results to return |
| `-s, --strategy` | `rrf` | Fusion strategy: `average`, `maximum`, `rrf`, `weighted` |
| `--rrf-k` | `60` | RRF k parameter (only used with the `rrf` strategy) |
| `-f, --format` | `table` | Output format (`table`, `json`) |

Strategy aliases: `average` (also `avg`), `maximum` (also `max`), `rrf`
(default), `weighted`. The `weighted` strategy uses fixed weights
(`avg_weight=0.5`, `max_weight=0.3`, `hit_weight=0.2`) that are not configurable
from the CLI.

**`query batch-search` flags:** `-k, --top-k` (default `10`) and
`-f, --format` (default `table`). Output groups results per query:

```
Batch Search Results (query 1)
  1. ID: 1 (score: 1.0000)
     Payload: {"category":"tech","title":"Rust in action"}
  Total: 1 result(s)

Batch Search Results (query 2)
  1. ID: 2 (score: 1.0000)
     Payload: {"category":"math","title":"Graph theory"}
  Total: 1 result(s)
```

**`query explain` formats:** `tree` (default, human-readable plan) or `json`.

```
Plan:
  Scan: docs
    Filter: category = 'tech'
    Limit: 10
  Estimated cost: 0.150 ms
```

---

## Analyze

```bash
# Collection statistics (point count, deletion ratio, index stats, column stats)
velesdb collection analyze ./data my_vectors
velesdb collection analyze ./data my_vectors --format json
```

---

## Graph

All graph subcommands take a database path and a graph collection name.

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

# Remove an edge by ID
velesdb graph remove-edge ./data my_graph 1

# Count edges and nodes
velesdb graph count ./data my_graph
velesdb graph count ./data my_graph --format json

# Search graph nodes by embedding similarity (requires a graph with embeddings)
velesdb graph search ./data my_graph '[0.1, 0.2, 0.3]' -k 10
velesdb graph search ./data my_graph '[0.1, 0.2, 0.3]' --format json

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

### `graph doctor` — legacy phantom-edge audit

Audits edge-store entries whose source or target node has no stored payload.
This can only happen in a database created before the #1442 `add_edge`
referential-integrity fix. Read-only by default; `--purge` and `--stub` are
mutually exclusive and idempotent.

```bash
velesdb graph doctor ./data my_graph                 # dry-run report
velesdb graph doctor ./data my_graph --purge         # remove phantom edges
velesdb graph doctor ./data my_graph --stub          # seed a {} payload for each missing endpoint
velesdb graph doctor ./data my_graph --format json
```

`graph doctor` is CLI-only — it has no REPL equivalent. See
[GRAPH_PATTERNS.md](GRAPH_PATTERNS.md).

> **Naming note:** the REPL uses `.graph edges` while the CLI uses
> `graph get-edges`. Both do the same thing.

---

## Index management

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

`--label` is required for the `property` and `range` index types.

---

## SIMD diagnostics

```bash
# Show SIMD dispatch configuration
velesdb simd info

# Force re-benchmark of all SIMD backends
velesdb simd benchmark
```

See [SIMD_PERFORMANCE.md](../reference/SIMD_PERFORMANCE.md).

---

## License management

```bash
# Show current license status
velesdb license show

# Activate a license
velesdb license activate <license_key>

# Verify a license without activating it
velesdb license verify <license_key> --public-key <base64_public_key>
```

License keys have the form `base64_payload.base64_signature` (Ed25519).
`license show` and `license activate` read the public key from
`VELESDB_LICENSE_PUBLIC_KEY`; when the variable is unset, a development
fallback key is used and a warning is printed. Set it with:

```bash
export VELESDB_LICENSE_PUBLIC_KEY=<base64_encoded_public_key>
```

---

## Shell completions

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

---

## Building the Debian package

The crate carries a `cargo-deb` configuration
(`[package.metadata.deb]` in `crates/velesdb-cli/Cargo.toml`). It packages the
`velesdb`, `velesdb-server` and `velesdb-migrate` release binaries, so build
them first:

```bash
cargo build --release -p velesdb-cli -p velesdb-server -p velesdb-migrate
cargo deb -p velesdb-cli
sudo dpkg -i target/debian/velesdb-cli_*.deb
```

The binary is installed as `velesdb` in `/usr/bin/`. Prebuilt `.deb` files are
attached to each GitHub release — see [INSTALLATION.md](INSTALLATION.md).

---

## Error reference

Errors actually emitted by the CLI, with their cause and remedy.

| Error | Cause | Fix |
|-------|-------|-----|
| `Collection 'X' not found` | The collection does not exist, or you used a command that expects a specific type (e.g. `data upsert` requires a vector collection but 'X' is a graph collection). | Check the name with `velesdb collection list ./data` or `.collections`. Use the command matching the collection type. |
| `Vector collection 'X' not found` | A vector-specific command (`data upsert`, `data delete`) was used on a graph or metadata collection. `data get` reports the same situation as `Collection 'X' not found`. | Use the correct collection type, or create a vector collection. |
| `Vector collection 'X' not found. Export requires a vector collection.` | `data export` was pointed at a graph or metadata collection. | Export only supports vector collections. Use `.export` in the REPL for metadata collections. |
| `Graph collection 'X' not found` | A graph command was used but the collection is not a graph collection. | Verify the type with `.schema X`, or create it with `collection create-graph`. |
| `[VELES-004] Vector dimension mismatch: expected N, got M` | The vector length does not match the collection's configured dimension. | Fix the vector length. During import, mismatched lines are skipped and counted in `errors`. |
| `Failed to open database` | The database path is incorrect, the directory does not exist, or permissions are insufficient. | Verify the path exists and is writable. |
| `null` from `data get` | The requested point ID does not exist. | `data get` prints `null` in JSON format, or `❌ Point with ID X not found` in table format. |
| `Parse error: …` / `= expected …` | Invalid VelesQL syntax. | Compare against the [VelesQL spec](../VELESQL_SPEC.md). |
| `Empty file` | The import file contains no records. | Verify the file is not empty and uses a supported format. |
| `Unsupported file format: X. Use .csv, .jsonl, or .bin (VRB1)` | `data import` received an unsupported extension. | Convert or rename the file. |
| `config file not found` | `--config` / `VELESDB_CONFIG` points at a missing file. | Fix the path — the CLI fails fast instead of falling back to defaults. |

Numeric `VELES-*` codes are listed in
[ERROR_CODES.md](../reference/ERROR_CODES.md).

---

Last updated: 2026-07-25 · Applies to: velesdb-core 4.2.0
