# velesdb-migrate

Migration tool for importing vectors from other databases into VelesDB.

## 🎯 Purpose

Switch to VelesDB in minutes, not days. `velesdb-migrate` handles the heavy lifting of extracting vectors from your current database and loading them into VelesDB with minimal configuration.

## 🚀 Try VelesDB Today!

> **Why migrate to VelesDB?**
> 
> - ⚡ **Microsecond latency** — 10-100x faster than cloud vector databases
> - 🎯 **SQL-native queries** — Use familiar VelesQL syntax, no new APIs to learn
> - 💾 **4-32x compression** — SQ8 and Binary quantization built-in
> - 🔒 **Self-hosted** — Your data stays on your infrastructure
> - 📦 **Single binary** — Zero dependencies, zero configuration
>
> ```bash
> # Quick test after migration
> velesdb query "SELECT * FROM my_collection ORDER BY vector <-> [0.1, 0.2, ...] LIMIT 10"
> ```

---

## ✅ Supported Sources

| Source | Status | Protocol | Notes |
|--------|--------|----------|-------|
| **Supabase** | ✅ Ready | PostgREST | pgvector via Supabase API |
| **PostgreSQL/pgvector** | ✅ Ready | SQL | Direct SQL connection |
| **Qdrant** | ✅ Ready | REST API | Scroll pagination |
| **Pinecone** | ✅ Ready | REST API | Serverless & pod indexes |
| **Weaviate** | ✅ Ready | GraphQL | All classes & properties |
| **Milvus** | ✅ Ready | REST API v2 | Zilliz Cloud compatible |
| **ChromaDB** | ✅ Ready | REST API | Tenant/database support |

---

## 🚀 Quick Start

### Installation

```bash
# From source
cargo install --path crates/velesdb-migrate

# With PostgreSQL support
cargo install --path crates/velesdb-migrate --features postgres
```

### Basic Usage

```bash
# 1. Generate config template for your source
velesdb-migrate init --source supabase --output migration.yaml

# 2. Edit configuration with your credentials
code migration.yaml

# 3. Validate configuration
velesdb-migrate validate --config migration.yaml

# 4. Preview source schema
velesdb-migrate schema --config migration.yaml

# 5. Run migration (dry run first!)
velesdb-migrate run --config migration.yaml --dry-run

# 6. Run actual migration
velesdb-migrate run --config migration.yaml
```

### 🔍 NEW: Auto-Detect Schema (Recommended)

**Skip manual configuration!** The `detect` command automatically:
- Connects to your source database
- Detects vector dimension (e.g., 1536 for OpenAI, 768 for sentence-transformers)
- Identifies vector and metadata columns
- Generates a ready-to-use YAML config

```bash
# Auto-detect from Supabase
velesdb-migrate detect \
  --source supabase \
  --url https://YOUR_PROJECT.supabase.co \
  --collection your_table \
  --api-key $SUPABASE_SERVICE_KEY \
  --output migration.yaml

# Auto-detect from Qdrant
velesdb-migrate detect \
  --source qdrant \
  --url http://localhost:6333 \
  --collection my_vectors \
  --output migration.yaml

# Auto-detect from ChromaDB
velesdb-migrate detect \
  --source chromadb \
  --url http://localhost:8000 \
  --collection embeddings \
  --output migration.yaml
```

**Example output:**
```
✅ Schema Detected!
┌─────────────────────────────────────────────
│ Source Type:  supabase
│ Collection:   documents
│ Dimension:    1536                    ← Auto-detected!
│ Total Count:  14053 vectors
├─────────────────────────────────────────────
│ Detected Metadata Fields:
│   • title (string)
│   • content (string)
│   • created_at (string)
└─────────────────────────────────────────────

📝 Configuration generated: "migration.yaml"
```

---

## 📚 Migration Guides by Source

### 🟢 Supabase (PostgREST API)

Supabase uses pgvector under the hood. Migration is done via the PostgREST API.

**Prerequisites:**
- Supabase project URL
- Service role key (for full access) or anon key (if RLS allows)
- Table with a vector column

**Configuration:**

```yaml
source:
  type: supabase
  url: https://YOUR_PROJECT_ID.supabase.co
  api_key: ${SUPABASE_SERVICE_KEY}  # Use env var for security
  table: documents
  vector_column: embedding          # Column containing the vector
  id_column: id                     # Primary key column
  payload_columns:                  # Additional columns to migrate
    - title
    - content
    - metadata
    - created_at

destination:
  path: ./velesdb_data
  collection: documents
  dimension: 1536                   # Must match your embedding model
  metric: cosine                    # cosine, euclidean, or dot
  storage_mode: full                # full, sq8 (4x compression), or binary (32x)

options:
  batch_size: 500                   # Supabase has row limits
  workers: 2
  continue_on_error: false
```

**Example Supabase Table Structure:**

```sql
CREATE TABLE documents (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  title TEXT,
  content TEXT,
  embedding VECTOR(1536),  -- OpenAI ada-002
  created_at TIMESTAMPTZ DEFAULT NOW()
);
```

---

### 🐘 PostgreSQL with pgvector (Direct SQL)

Direct SQL connection for self-hosted PostgreSQL with pgvector extension.

**Prerequisites:**
- PostgreSQL connection string
- pgvector extension installed
- Compile with `--features postgres`

**Configuration:**

```yaml
source:
  type: pgvector
  connection_string: postgres://user:password@localhost:5432/mydb
  table: embeddings
  vector_column: embedding
  id_column: id
  payload_columns:
    - title
    - content
    - category
  filter: "created_at > '2024-01-01'"  # Optional WHERE clause

destination:
  path: ./velesdb_data
  collection: pg_documents
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

**Installation with PostgreSQL support:**

```bash
cargo install --path crates/velesdb-migrate --features postgres
```

---

### 🔵 Qdrant

Full support for Qdrant Cloud and self-hosted instances.

**Prerequisites:**
- Qdrant URL (default: `http://localhost:6333`)
- API key (for Qdrant Cloud)
- Collection name

**Configuration:**

```yaml
source:
  type: qdrant
  url: http://localhost:6333
  # url: https://xxx-xxx.aws.cloud.qdrant.io  # For Qdrant Cloud
  collection: my_collection
  api_key: ${QDRANT_API_KEY}        # Optional, for cloud
  payload_fields: []                 # Empty = all fields

destination:
  path: ./velesdb_data
  collection: qdrant_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
  workers: 4
```

**Features Supported:**
- ✅ Numeric and UUID point IDs
- ✅ Single and named vectors
- ✅ All payload types
- ✅ Scroll pagination (efficient for large collections)

---

### 🌲 Pinecone

Supports both serverless and pod-based Pinecone indexes.

**Prerequisites:**
- Pinecone API key
- Index name
- Optional: namespace

**Configuration:**

```yaml
source:
  type: pinecone
  api_key: ${PINECONE_API_KEY}
  environment: us-east-1-aws        # Your Pinecone environment
  index: my-index
  namespace: production             # Optional

destination:
  path: ./velesdb_data
  collection: pinecone_vectors
  dimension: 1536
  metric: cosine

options:
  batch_size: 100                   # Pinecone has lower limits
  workers: 2
```

**Notes:**
- Pinecone API has rate limits, use smaller batch sizes
- Namespaces are optional but recommended for organization

---

### 🟠 Weaviate

GraphQL-based extraction from Weaviate instances.

**Prerequisites:**
- Weaviate URL
- Class name
- Optional: API key (for Weaviate Cloud)

**Configuration:**

```yaml
source:
  type: weaviate
  url: http://localhost:8080
  # url: https://xxx.weaviate.network  # For Weaviate Cloud
  class_name: Document
  api_key: ${WEAVIATE_API_KEY}      # Optional
  properties:                        # Properties to include
    - title
    - content
    - author

destination:
  path: ./velesdb_data
  collection: weaviate_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

**Features Supported:**
- ✅ All property types
- ✅ Cursor-based pagination
- ✅ GraphQL query optimization

---

### 🔷 Milvus / Zilliz Cloud

REST API v2 support for Milvus and Zilliz Cloud.

**Prerequisites:**
- Milvus URL (default: `http://localhost:19530`)
- Collection name
- Optional: username/password

**Configuration:**

```yaml
source:
  type: milvus
  url: http://localhost:19530
  # url: https://xxx.zillizcloud.com  # For Zilliz Cloud
  collection: my_collection
  username: root                     # Optional
  password: ${MILVUS_PASSWORD}       # Optional

destination:
  path: ./velesdb_data
  collection: milvus_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

---

### 🟡 ChromaDB

Full support for ChromaDB instances with tenant/database isolation.

**Prerequisites:**
- ChromaDB URL (default: `http://localhost:8000`)
- Collection name

**Configuration:**

```yaml
source:
  type: chromadb
  url: http://localhost:8000
  collection: my_collection
  tenant: default_tenant             # Optional
  database: default_database         # Optional

destination:
  path: ./velesdb_data
  collection: chroma_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

**Features Supported:**
- ✅ Embeddings extraction
- ✅ Metadata migration
- ✅ Document content
- ✅ Multi-tenant support

---

## 🔧 CLI Reference

```
velesdb-migrate 0.5.2
Migrate vectors from other databases to VelesDB

USAGE:
    velesdb-migrate [OPTIONS] [COMMAND]

COMMANDS:
    run       Run migration from config file
    validate  Validate configuration file
    schema    Show schema from source database
    init      Generate example configuration
    detect    Auto-detect schema and generate config (NEW!)

OPTIONS:
    -c, --config <FILE>     Configuration file path
        --dry-run           Preview migration without writing
    -v, --verbose           Verbose output (debug logs)
        --batch-size <N>    Override batch size from config
    -h, --help              Print help
    -V, --version           Print version
```

### Detect Command Options

```
velesdb-migrate detect [OPTIONS]

OPTIONS:
    -s, --source <TYPE>      Source type: supabase, qdrant, chromadb, pinecone, weaviate, milvus
    -u, --url <URL>          Source database URL
    -n, --collection <NAME>  Collection/table/index name
    -a, --api-key <KEY>      API key (required for some sources)
    -o, --output <FILE>      Output config file [default: migration.yaml]
        --dest-path <PATH>   VelesDB destination path [default: ./velesdb_data]
```

### Command Examples

```bash
# Generate config for each source type
velesdb-migrate init --source supabase --output supabase.yaml
velesdb-migrate init --source pgvector --output pgvector.yaml
velesdb-migrate init --source qdrant --output qdrant.yaml
velesdb-migrate init --source pinecone --output pinecone.yaml
velesdb-migrate init --source weaviate --output weaviate.yaml
velesdb-migrate init --source milvus --output milvus.yaml
velesdb-migrate init --source chromadb --output chromadb.yaml

# Check source schema before migration
velesdb-migrate schema --config migration.yaml

# Dry run (recommended before actual migration)
velesdb-migrate run --config migration.yaml --dry-run

# Full migration with verbose output
velesdb-migrate run --config migration.yaml --verbose

# Override batch size for testing
velesdb-migrate run --config migration.yaml --batch-size 100
```

---

## ⚙️ Configuration Options

### Destination Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `path` | string | required | Path to VelesDB data directory |
| `collection` | string | required | Collection name (created if not exists) |
| `dimension` | integer | required | Vector dimension (must match source) |
| `metric` | string | `cosine` | Distance metric: `cosine`, `euclidean`, `dot` |
| `storage_mode` | string | `full` | Storage: `full`, `sq8` (4x compression), `binary` (32x) |

### Migration Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `batch_size` | integer | `1000` | Points extracted per batch |
| `workers` | integer | `4` | Parallel workers (not yet implemented) |
| `checkpoint_enabled` | boolean | `true` | Enable resume support |
| `checkpoint_path` | string | auto | Custom checkpoint file path |
| `dry_run` | boolean | `false` | Preview only, don't write |
| `continue_on_error` | boolean | `false` | Skip failed points |
| `field_mappings` | map | `{}` | Rename fields during migration |

### Field Mappings Example

Rename fields during migration:

```yaml
options:
  field_mappings:
    old_field_name: new_field_name
    legacy_title: title
    doc_content: content
    created: created_at
```

---

## 📊 Performance Guidelines

### Expected Throughput

| Source | Typical Speed | Recommended Batch Size |
|--------|--------------|------------------------|
| Local Qdrant | 10,000+ pts/sec | 1000 |
| Cloud Qdrant | 1,000-5,000 pts/sec | 500-1000 |
| Supabase | 1,000-3,000 pts/sec | 500 |
| Pinecone | 500-2,000 pts/sec | 100 |
| Weaviate | 2,000-5,000 pts/sec | 1000 |
| Milvus | 3,000-8,000 pts/sec | 1000 |
| ChromaDB | 2,000-5,000 pts/sec | 1000 |
| pgvector (local) | 5,000-15,000 pts/sec | 1000 |

### Optimization Tips

1. **Start with dry run**: Always preview first
2. **Use smaller batches for cloud sources**: API rate limits apply
3. **Monitor memory usage**: Large batches use more RAM
4. **Use SQ8 storage**: 4x memory reduction with ~99% recall
5. **Enable checkpoints**: For large migrations, allows resume

---

## 🔐 Security Best Practices

### Environment Variables

Never hardcode secrets in config files:

```yaml
source:
  api_key: ${MY_API_KEY}  # Reads from environment
```

```bash
export MY_API_KEY="your-secret-key"
velesdb-migrate run --config migration.yaml
```

### Recommended Permissions

| Source | Recommended Permission |
|--------|----------------------|
| Supabase | Service role key (read-only if possible) |
| Qdrant | Read-only API key |
| Pinecone | Read-only API key |
| Weaviate | Read-only auth token |
| pgvector | SELECT permission on tables |

### .gitignore

```
# Never commit migration configs with secrets
migration.yaml
*.migration.yaml
.env
```

---

## 🐛 Troubleshooting

### Connection Errors

```
Error: Source connection error: ...
```

**Solutions:**
- Verify URL format (include protocol: `http://` or `https://`)
- Check network connectivity
- Verify credentials
- Ensure collection/table exists

### Dimension Mismatch

```
Error: Schema mismatch: Source dimension 768 != destination dimension 1536
```

**Solutions:**
- Check your embedding model's dimension
- Update `dimension` in destination config
- Run `velesdb-migrate schema` to see source dimension

### Rate Limit Errors

```
Error: Rate limit exceeded, retry after 60 seconds
```

**Solutions:**
- Reduce `batch_size` (try 100 or 50)
- Add delays between batches (coming soon)
- Check source API quotas

### Memory Issues

```
Error: Out of memory
```

**Solutions:**
- Reduce `batch_size`
- Use `storage_mode: sq8` for 4x memory reduction
- Process in smaller chunks

### Resume Failed Migration

If migration fails midway:

```bash
# The checkpoint file stores progress
# Just re-run the same command
velesdb-migrate run --config migration.yaml

# Or start fresh by removing checkpoint
rm .velesdb_migrate_checkpoint.json
velesdb-migrate run --config migration.yaml
```

---

## 📁 Example Configuration Files

See the `examples/` directory for complete configuration templates:

- `examples/qdrant-migration.yaml`
- `examples/pinecone-migration.yaml`
- `examples/weaviate-migration.yaml`
- `examples/milvus-migration.yaml`
- `examples/chromadb-migration.yaml`
- `examples/supabase-migration.yaml`

---

## 🔄 Migration Workflow

### Option A: Auto-Detect (Recommended) ⚡

```
┌─────────────────────────────────────────────────────────────┐
│              FAST WORKFLOW (Auto-Detect)                     │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. DETECT                                                   │
│     velesdb-migrate detect --source supabase --url ...       │
│     → Auto-detects dimension, columns, count                 │
│     → Generates migration.yaml                               │
│                          │                                   │
│                          ▼                                   │
│  2. REVIEW                                                   │
│     Verify generated config (optional adjustments)           │
│                          │                                   │
│                          ▼                                   │
│  3. MIGRATE                                                  │
│     velesdb-migrate run --config migration.yaml              │
│                          │                                   │
│                          ▼                                   │
│  4. DONE! ✅                                                  │
│     velesdb query "SELECT COUNT(*) FROM collection"          │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

### Option B: Manual Configuration

```
┌─────────────────────────────────────────────────────────────┐
│                 MANUAL WORKFLOW                              │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  1. INIT                                                     │
│     velesdb-migrate init --source <type> --output config.yaml│
│                          │                                   │
│                          ▼                                   │
│  2. CONFIGURE                                                │
│     Edit config.yaml with credentials                        │
│                          │                                   │
│                          ▼                                   │
│  3. VALIDATE                                                 │
│     velesdb-migrate validate --config config.yaml            │
│                          │                                   │
│                          ▼                                   │
│  4. SCHEMA                                                   │
│     velesdb-migrate schema --config config.yaml              │
│     → Shows dimension, count, fields                         │
│                          │                                   │
│                          ▼                                   │
│  5. DRY RUN                                                  │
│     velesdb-migrate run --config config.yaml --dry-run       │
│     → Validates without writing                              │
│                          │                                   │
│                          ▼                                   │
│  6. MIGRATE                                                  │
│     velesdb-migrate run --config config.yaml                 │
│     → Extracts → Transforms → Loads                          │
│                          │                                   │
│                          ▼                                   │
│  7. VERIFY                                                   │
│     velesdb query "SELECT COUNT(*) FROM collection"          │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

---

## 📐 Dimension Detection by Source

All connectors automatically detect vector dimensions:

| Source | Detection Method | Reliability |
|--------|------------------|-------------|
| **Supabase** | Fetch 1 row, parse pgvector format | ✅ 100% |
| **PostgreSQL/pgvector** | Query vector column | ✅ 100% |
| **Qdrant** | Collection info API | ✅ 100% |
| **Pinecone** | Index stats API | ✅ 100% |
| **Weaviate** | GraphQL fetch 1 vector | ✅ 100% |
| **Milvus** | Schema field type | ✅ 100% |
| **ChromaDB** | Fetch 1 embedding | ✅ 100% |

**Common dimensions:**
- `1536` — OpenAI text-embedding-ada-002, text-embedding-3-small/large
- `768` — Sentence-transformers all-mpnet-base-v2
- `384` — Sentence-transformers all-MiniLM-L6-v2
- `1024` — Cohere embed-english-v3.0
- `3072` — OpenAI text-embedding-3-large (full)

---

## 🇫🇷 About

Developed by **Wiscale France** (Julien Lange).

Part of the VelesDB project — **Vector Search in Microseconds**.

---

## 🚀 Ready to Try VelesDB?

```bash
# 1. Install VelesDB
cargo install velesdb

# 2. Migrate your data
velesdb-migrate detect --source qdrant --url http://localhost:6333 --collection my_data
velesdb-migrate run --config migration.yaml

# 3. Query with VelesQL (SQL-native!)
velesdb query "SELECT id, title FROM my_data ORDER BY vector <-> [0.1, 0.2, ...] LIMIT 10"

# 4. Start the REST API server
velesdb serve --port 8080
```

### Why developers choose VelesDB:

- ✅ **10-100x faster** than cloud vector DBs
- ✅ **SQL syntax** you already know  
- ✅ **Single binary**, no dependencies
- ✅ **Self-hosted**, your data stays private
- ✅ **4-32x compression** with SQ8/Binary quantization

📚 **Learn more:** [github.com/velesdb/velesdb](https://github.com/velesdb/velesdb)

---

## 📄 License

ELv2 (Elastic License 2.0) — Same as VelesDB Core.
