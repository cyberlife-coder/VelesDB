# VelesDB CLI

[![Crates.io](https://img.shields.io/crates/v/velesdb-cli.svg)](https://crates.io/crates/velesdb-cli)
[![License](https://img.shields.io/badge/license-ELv2-blue)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)

Interactive CLI and REPL for VelesDB with VelesQL support.

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

## Usage

### Interactive REPL

```bash
# Start interactive REPL with default data directory (./data)
velesdb repl

# Or specify a data directory
velesdb repl ./my_vectors
```

### Single Query Execution

```bash
# Execute a query directly
velesdb query ./my_vectors "SELECT * FROM documents LIMIT 10"

# Output as JSON
velesdb query ./my_vectors "SELECT * FROM documents LIMIT 5" --format json
```

### Database Info

```bash
velesdb info ./my_vectors
```

### List Collections

```bash
# List all collections
velesdb list ./my_vectors

# Output as JSON
velesdb list ./my_vectors --format json
```

### Show Collection Details

```bash
# Show collection schema and stats
velesdb show ./my_vectors documents

# Show with sample records
velesdb show ./my_vectors documents --samples 5
```

### Import/Export

```bash
# Export collection to JSON
velesdb export ./my_vectors documents --output documents.json

# Export without vectors (metadata only)
velesdb export ./my_vectors documents --output meta.json --include-vectors false

# Import from JSONL file
velesdb import data.jsonl --database ./my_vectors --collection documents --dimension 768

# Import from CSV
velesdb import embeddings.csv --database ./my_vectors --collection docs --dimension 384
```

### VelesQL Queries

#### Vector Search

```sql
-- Vector similarity search (uses collection's metric)
SELECT * FROM documents 
WHERE VECTOR NEAR [0.15, 0.25, ...] 
LIMIT 5;

-- With parameter (for API usage)
SELECT * FROM documents 
WHERE VECTOR NEAR $query_vector
LIMIT 10;
```

> ℹ️ **Note**: The distance metric is defined at **collection creation time** and cannot be changed per-query. All 5 metrics (Cosine, Euclidean, DotProduct, Hamming, Jaccard) are supported.

#### Metadata Filtering

Metadata is stored as JSON. Query any field with SQL operators:

```sql
-- Equality
SELECT * FROM docs WHERE category = 'tech' LIMIT 10;

-- Numeric comparisons
SELECT * FROM docs WHERE views > 1000 LIMIT 10;
SELECT * FROM docs WHERE price >= 50 AND price <= 200 LIMIT 10;

-- String patterns
SELECT * FROM docs WHERE title LIKE '%rust%' LIMIT 10;

-- IN list
SELECT * FROM docs WHERE status IN ('published', 'featured') LIMIT 10;

-- BETWEEN range
SELECT * FROM docs WHERE score BETWEEN 0.5 AND 1.0 LIMIT 10;

-- NULL checks
SELECT * FROM docs WHERE author IS NOT NULL LIMIT 10;

-- Full-text search (BM25)
SELECT * FROM docs WHERE content MATCH 'rust programming' LIMIT 10;
```

#### Multi-Query Fusion (MQG) ⭐ NEW v1.1.0

```sql
-- RRF fusion with multiple query vectors (ideal for RAG pipelines)
SELECT * FROM documents 
WHERE VECTOR NEAR_FUSED [$v1, $v2, $v3]
WITH (fusion = 'rrf', k = 60)
LIMIT 10;

-- Weighted fusion strategy
SELECT * FROM documents 
WHERE VECTOR NEAR_FUSED [$query1, $query2]
WITH (fusion = 'weighted', avg_weight = 0.6, max_weight = 0.3, hit_weight = 0.1)
LIMIT 10;

-- Average/Maximum fusion
SELECT * FROM documents 
WHERE VECTOR NEAR_FUSED $vectors
WITH (fusion = 'average')
LIMIT 10;
```

#### Combined Queries (Vector + Metadata + Text)

```sql
-- Vector search + metadata filter
SELECT * FROM documents 
WHERE VECTOR NEAR [0.15, 0.25, ...] 
AND category = 'tech' 
AND views > 100
LIMIT 5;

-- Hybrid search (vector + full-text)
SELECT * FROM documents
WHERE VECTOR NEAR $query 
AND content MATCH 'rust'
LIMIT 5;

-- Complex conditions
SELECT * FROM products
WHERE VECTOR NEAR COSINE [0.1, 0.2, ...]
AND (category = 'electronics' OR category = 'gadgets')
AND price BETWEEN 100 AND 500
AND in_stock = true
LIMIT 10;
```

#### WITH Clause (Query Options)

Override search parameters per-query:

```sql
-- Set search mode (fast, balanced, accurate, high_recall, perfect)
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (mode = 'high_recall');

-- Set ef_search parameter directly
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (ef_search = 512);

-- Multiple options
SELECT * FROM docs WHERE VECTOR NEAR $v LIMIT 10
WITH (mode = 'fast', timeout_ms = 5000, rerank = true);
```

| Option | Type | Description |
|--------|------|-------------|
| `mode` | string | Search preset: fast, balanced, accurate, high_recall, perfect |
| `ef_search` | integer | HNSW ef_search parameter (higher = better recall) |
| `timeout_ms` | integer | Query timeout in milliseconds |
| `rerank` | boolean | Enable result reranking |

#### Available Filter Operators

| Operator | Syntax | Example |
|----------|--------|---------|
| Equal | `=` | `status = 'active'` |
| Not Equal | `!=`, `<>` | `type != 'draft'` |
| Greater | `>` | `views > 1000` |
| Greater/Equal | `>=` | `rating >= 4` |
| Less | `<` | `price < 100` |
| Less/Equal | `<=` | `age <= 30` |
| IN | `IN (...)` | `tag IN ('a', 'b')` |
| BETWEEN | `BETWEEN...AND` | `score BETWEEN 0 AND 1` |
| LIKE | `LIKE` | `name LIKE '%john%'` |
| IS NULL | `IS NULL` | `email IS NULL` |
| IS NOT NULL | `IS NOT NULL` | `phone IS NOT NULL` |
| Full-text | `MATCH` | `body MATCH 'search'` |

### Dot Commands

| Command | Description |
|---------|-------------|
| `.help` | Show help |
| `.quit` / `.exit` | Exit REPL |
| `.collections` / `.tables` | List collections |
| `.schema <name>` | Show collection schema |
| `.describe <name>` | Show detailed collection info |
| `.count <name>` | Show point count |
| `.timing on/off` | Toggle query timing |
| `.format table/json` | Set output format |
| `.clear` | Clear screen |

## Features

- **VelesQL Support**: SQL-like syntax for vector operations
- **Tab Completion**: Auto-complete collection names and keywords
- **Command History**: Arrow keys to navigate history
- **Colored Output**: Easy-to-read formatted results
- **Timing**: Query execution time display

## Examples

### Semantic Search

```sql
-- Search with metadata filter
SELECT id, score, payload->title FROM articles
WHERE VECTOR NEAR $query_embedding
AND category = 'technology'
LIMIT 5;

-- Search with multiple conditions
SELECT * FROM documents
WHERE VECTOR NEAR [0.1, 0.2, 0.3, ...]
AND status = 'published'
AND views > 1000
LIMIT 10;
```

### Binary Vector Search

```sql
-- Find similar binary vectors (fingerprints, hashes)
SELECT * FROM images
WHERE VECTOR NEAR [1.0, 0.0, 1.0, 1.0, 0.0, ...]
LIMIT 10;
```

### Creating Collections (via Rust API)

Collections are created programmatically, not via VelesQL:

```rust
use velesdb_core::{Database, DistanceMetric};

let db = Database::open("./data")?;
db.create_collection("articles", 384, DistanceMetric::Cosine)?;
db.create_collection("images", 256, DistanceMetric::Hamming)?;
```

## License

Elastic License 2.0 (ELv2)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
