# VelesDB CLI & REPL Reference

> Interactive command-line interface and REPL for VelesDB with VelesQL support.

## Installation

```bash
# From crates.io
cargo install velesdb-cli

# From source
cargo install --path crates/velesdb-cli
```

## CLI Commands

### Start Interactive REPL

```bash
velesdb repl ./my_database
```

### Execute Single Query

```bash
velesdb query ./my_database "SELECT * FROM docs LIMIT 10"
velesdb query ./my_database "SELECT * FROM docs LIMIT 10" --format json
```

### Database Information

```bash
velesdb info ./my_database
velesdb list ./my_database
velesdb show ./my_database my_collection --samples 5
```

### Import Data

```bash
# Import from JSONL
velesdb import ./data.jsonl --database ./db --collection docs

# Import from CSV with options
velesdb import ./vectors.csv \
  --database ./db \
  --collection embeddings \
  --dimension 768 \
  --metric cosine \
  --storage-mode sq8 \
  --batch-size 1000
```

**Import options:**
- `--dimension`: Vector dimension (auto-detected if not specified)
- `--metric`: Distance metric (`cosine`, `euclidean`, `dot`, `hamming`, `jaccard`)
- `--storage-mode`: Storage mode (`full`, `sq8`, `binary`)
- `--id-column`: ID column name for CSV (default: `id`)
- `--vector-column`: Vector column name for CSV (default: `vector`)
- `--batch-size`: Batch size for insertion (default: 1000)

### Export Data

```bash
velesdb export ./my_database my_collection --output backup.json
```

### License Management

```bash
velesdb license show
velesdb license activate <LICENSE_KEY>
velesdb license verify <LICENSE_KEY> --public-key <BASE64_KEY>
```

## REPL Commands

Once in the REPL (`velesdb repl ./db`), use these commands:

### Navigation

| Command | Description |
|---------|-------------|
| `.help` | Show help |
| `.quit` / `.exit` | Exit REPL |
| `.clear` | Clear screen |

### Database Exploration

| Command | Description |
|---------|-------------|
| `.collections` | List all collections |
| `.schema <name>` | Show collection schema |
| `.describe <name>` | Detailed collection stats |
| `.count <name>` | Count records |
| `.sample <name> [n]` | Show N sample records |
| `.browse <name> [page]` | Browse with pagination |

### Data Management

| Command | Description |
|---------|-------------|
| `.export <name> [file]` | Export to JSON file |

### Output Control

| Command | Description |
|---------|-------------|
| `.timing on\|off` | Toggle timing display |
| `.format table\|json` | Set output format |

### Session Commands

| Command | Description |
|---------|-------------|
| `\set <key> <value>` | Set session parameter |
| `\show [key]` | Show session settings |
| `\reset [key]` | Reset settings |
| `\use <collection>` | Select active collection |
| `\info` | Database information |
| `\bench <col> [n] [k]` | Quick benchmark |

### Session Settings

| Setting | Values | Default | Description |
|---------|--------|---------|-------------|
| `mode` | `fast`, `balanced`, `accurate`, `high_recall`, `perfect` | `balanced` | Search mode |
| `ef_search` | 16-4096 | auto | HNSW ef parameter |
| `timeout_ms` | >=100 | 30000 | Query timeout |
| `rerank` | `true`/`false` | `true` | Enable reranking |
| `max_results` | 1-10000 | 100 | Max results per query |

## VelesQL Examples in REPL

```sql
-- Basic select
SELECT * FROM documents LIMIT 10;

-- Vector similarity search
SELECT * FROM docs WHERE vector NEAR $v LIMIT 5 WITH (mode = 'fast');

-- Filtered search
SELECT * FROM items WHERE category = 'tech' LIMIT 20;
```

## History

Command history is saved to `~/.local/share/.velesdb_history` (Linux/macOS) or `%LOCALAPPDATA%\.velesdb_history` (Windows).

## Environment Variables

| Variable | Description |
|----------|-------------|
| `VELESDB_LICENSE_PUBLIC_KEY` | Base64-encoded public key for license validation |

## Examples

### Quick Start Session

```bash
$ velesdb repl ./my_data

VelesDB v0.8.10 - VelesQL REPL
Database: ./my_data
Type .help for commands, .quit to exit

velesdb> .collections
Collections:
  - documents

velesdb> .describe documents
Collection Details
  Name: documents
  Dimension: 768
  Metric: Cosine
  Point Count: 10000
  Storage Mode: Full
  Est. Memory: 30.72 MB

velesdb> SELECT * FROM documents LIMIT 3;
+----+------------------+
| id | title            |
+----+------------------+
| 1  | Introduction     |
| 2  | Getting Started  |
| 3  | API Reference    |
+----+------------------+
3 rows (0.42ms)

velesdb> .quit
Goodbye!
```

## License

ELv2 (Elastic License 2.0)
