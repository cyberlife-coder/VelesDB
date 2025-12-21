# VelesDB CLI

[![Crates.io](https://img.shields.io/crates/v/velesdb-cli.svg)](https://crates.io/crates/velesdb-cli)
[![License](https://img.shields.io/crates/l/velesdb-cli.svg)](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE)

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
# Start interactive mode
velesdb

# Or specify a data directory
velesdb --data ./my_vectors
```

### REPL Commands

```sql
-- Create a collection
CREATE COLLECTION documents DIMENSION 768 METRIC cosine;

-- Insert vectors
INSERT INTO documents (id, vector, payload) VALUES 
  (1, [0.1, 0.2, ...], '{"title": "Hello"}');

-- Search for similar vectors
SELECT * FROM documents 
WHERE VECTOR NEAR [0.15, 0.25, ...] 
LIMIT 5;

-- Filter by metadata
SELECT * FROM documents 
WHERE category = 'tech' AND price > 100
LIMIT 10;

-- List collections
SHOW COLLECTIONS;

-- Get collection info
DESCRIBE documents;

-- Drop collection
DROP COLLECTION documents;
```

### Special Commands

| Command | Description |
|---------|-------------|
| `.help` | Show help |
| `.exit` | Exit REPL |
| `.tables` | List collections |
| `.schema <name>` | Show collection schema |
| `.timing on/off` | Toggle query timing |

## Features

- **VelesQL Support**: SQL-like syntax for vector operations
- **Tab Completion**: Auto-complete collection names and keywords
- **Command History**: Arrow keys to navigate history
- **Colored Output**: Easy-to-read formatted results
- **Timing**: Query execution time display

## Examples

### Semantic Search

```sql
-- Create collection for embeddings
CREATE COLLECTION articles DIMENSION 384 METRIC cosine;

-- Search with metadata filter
SELECT id, score, payload->title FROM articles
WHERE VECTOR NEAR $query_embedding
AND category = 'technology'
LIMIT 5;
```

### Binary Vector Search

```sql
-- Create collection for image fingerprints
CREATE COLLECTION images DIMENSION 256 METRIC hamming;

-- Find similar images
SELECT * FROM images
WHERE VECTOR NEAR [1, 0, 1, 1, 0, ...]
LIMIT 10;
```

## License

Business Source License 1.1 (BSL-1.1)

See [LICENSE](https://github.com/cyberlife-coder/velesdb/blob/main/LICENSE) for details.
