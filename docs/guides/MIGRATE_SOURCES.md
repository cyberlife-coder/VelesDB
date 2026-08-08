# velesdb-migrate — source reference

Per-source configuration for every connector shipped by
[`velesdb-migrate`](../../crates/velesdb-migrate/README.md).

Every field below is the field name that `serde` actually deserialises, taken
from `crates/velesdb-migrate/src/config.rs` and
`crates/velesdb-migrate/src/connectors/`. Unknown keys are silently ignored, so
a typo in a field name does **not** raise an error — it falls back to the
default. Check your run against `velesdb-migrate schema --config <file>` before
trusting a migration.

Related pages:

- [CLI and configuration reference](./MIGRATE_CLI.md)
- [Operations: performance, secrets, troubleshooting](./MIGRATE_OPERATIONS.md)

---

## Local URLs are rejected by default

Six connectors (`qdrant`, `weaviate`, `milvus`, `elasticsearch`, `redis`,
`supabase`) validate their `url` against an anti-SSRF policy before the first
request. Loopback, RFC 1918, link-local and reserved hostnames
(`localhost`, `*.local`, `*.internal`, `*.arpa`) are refused:

```text
Error: Configuration error: URL 'http://localhost:6333' targets reserved hostname 'localhost' (localhost / .local / .internal / .arpa). Set VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1 for local development.
```

Every `http://localhost:...` example on this page therefore needs the escape
hatch:

```bash
export VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1
```

`chromadb` and `pinecone` do not call the validator (Pinecone discovers its
host from the API), so they work against a local instance without the variable.

---

## Supabase (PostgREST over pgvector)

**Prerequisites:** project URL, service-role key (or anon key if RLS allows),
a table with a `vector` column.

```yaml
source:
  type: supabase
  url: https://your-project.supabase.co
  api_key: your-service-role-key
  table: documents
  vector_column: embedding      # default: embedding
  id_column: id                 # default: id
  payload_columns:              # empty/absent = none
    - title
    - content
  metric: vector_cosine_ops     # optional, see below

destination:
  path: ./velesdb_data
  collection: supabase_docs
  dimension: 1536
  metric: cosine
  storage_mode: full

options:
  batch_size: 500
  workers: 2
```

Matching table shape:

```sql
CREATE TABLE documents (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  title TEXT,
  content TEXT,
  embedding VECTOR(1536),
  created_at TIMESTAMPTZ DEFAULT NOW()
);
```

`source.metric` is optional and exists because PostgREST does not expose
`pg_catalog`, so the pgvector operator class cannot be introspected. Declare it
(`vector_cosine_ops` / `vector_l2_ops` / `vector_ip_ops`, or the VelesDB names
`cosine` / `euclidean` / `dot`) to enable the metric-fidelity check. Leaving it
unset logs a `WARN` on `get_schema` so the skipped check is never silent.

## Qdrant

**Prerequisites:** Qdrant URL, collection name, API key for Qdrant Cloud.

```yaml
source:
  type: qdrant
  url: https://xyz.aws.cloud.qdrant.io
  collection: my_collection
  api_key: your-qdrant-api-key   # optional (Cloud)
  payload_fields: []             # accepted but inert — see note below

destination:
  path: ./velesdb_data
  collection: qdrant_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
  workers: 4
```

Supported: numeric and UUID point IDs, single and named vectors, all payload
types, scroll pagination. Named sparse vectors are extracted (see the sparse
matrix below). A point that carries **only** a sparse vector is rejected — the
pipeline requires a dense vector per point.

`payload_fields` parses but the Qdrant connector never reads it: the scroll
request is always issued with `with_payload: true`, so the full payload is
migrated. Use `options.field_mappings` to rename fields, and prune afterwards
if you need a subset.

## Pinecone

**Prerequisites:** API key, index name, optional namespace.

```yaml
source:
  type: pinecone
  api_key: your-pinecone-api-key
  environment: us-east-1-aws     # deprecated since 1.12.0, kept for old configs
  index: your-index-name
  namespace: production          # optional

destination:
  path: ./velesdb_data
  collection: pinecone_vectors
  dimension: 1536
  metric: cosine

options:
  batch_size: 100                # Pinecone rate-limits aggressively
  workers: 2
```

`environment` is ignored by Pinecone serverless (2024+): the host is discovered
through `GET /indexes/{name}`. The field is retained only so pre-1.12 configs
keep parsing.

## Weaviate

**Prerequisites:** Weaviate URL, class name, optional API key.

```yaml
source:
  type: weaviate
  url: https://your-cluster.weaviate.network
  class_name: Document
  api_key: your-weaviate-api-key  # optional
  properties:                     # empty/absent = no payload migrated
    - title
    - content

destination:
  path: ./velesdb_data
  collection: weaviate_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

Supported: all property types, cursor-based pagination, GraphQL extraction.
The GraphQL selection set is built from `properties`; when the list is empty
only `_additional { id vector }` is requested, so the migrated points carry no
payload. List every property you want to keep.

## Milvus / Zilliz Cloud

**Prerequisites:** Milvus URL, collection name, optional credentials.

```yaml
source:
  type: milvus
  url: https://your-cluster.zillizcloud.com
  collection: my_collection
  username: root                  # optional
  password: your-password         # optional

destination:
  path: ./velesdb_data
  collection: milvus_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

Uses the Milvus REST API v2; Zilliz Cloud is compatible.

## ChromaDB

**Prerequisites:** ChromaDB URL, collection name.

```yaml
source:
  type: chromadb
  url: http://localhost:8000
  collection: my_collection
  tenant: default_tenant          # optional
  database: default_database      # optional

destination:
  path: ./velesdb_data
  collection: chroma_docs
  dimension: 768
  metric: cosine

options:
  batch_size: 1000
```

Supported: embeddings, metadata, document content, multi-tenant isolation.

## Elasticsearch / OpenSearch

**Prerequisites:** cluster URL, index with a `dense_vector` field, credentials
if security is on.

```yaml
source:
  type: elasticsearch
  url: https://your-cluster.example.com:9200
  index: vectors
  vector_field: embedding         # default: embedding
  id_field: _id                   # default: _id
  payload_fields: []              # empty = all except _id and the vector
  username: elastic               # optional (Basic auth)
  password: your-password         # optional
  api_key: your-api-key           # optional (alternative to Basic auth)
  query:                          # optional Elasticsearch DSL filter
    term:
      status: active

destination:
  path: ./velesdb_data
  collection: es_vectors
  dimension: 768
  metric: cosine

options:
  batch_size: 100
```

Pagination uses `search_after`. OpenSearch is compatible.

## Redis Stack (RediSearch)

Requires the default `redis-source` feature (enabled unless you build with
`--no-default-features`).

**Prerequisites:** Redis Stack with RediSearch, an index created via
`FT.CREATE`, and the key prefix that index covers.

```yaml
source:
  type: redis
  url: rediss://your-redis.example.com:6379
  password: your-password         # optional
  index: vectors_idx
  vector_field: embedding         # default: embedding
  key_prefix: "doc:"              # default: doc:
  payload_fields: []              # empty = all
  filter: "@status:{active}"      # optional RediSearch filter

destination:
  path: ./velesdb_data
  collection: redis_vectors
  dimension: 768
  metric: cosine

options:
  batch_size: 100
```

Allowed URL schemes are `redis` and `rediss`.

## JSON file

**Prerequisites:** a `.json` file whose root (or `array_path`) is an array of
objects.

```yaml
source:
  type: json_file
  path: ./vectors.json
  array_path: ""                  # "" = root array; else "data.items"
  id_field: id                    # default: id
  vector_field: vector            # default: vector
  payload_fields: []              # empty = every field except id and vector

destination:
  path: ./velesdb_data
  collection: json_vectors
  dimension: 4
  metric: cosine

options:
  batch_size: 1000
```

The vector may be a JSON array (`[0.1, 0.2]`) or a JSON array **encoded as a
string** (`"[0.1, 0.2]"`). A record with no usable `id_field` gets the
synthetic id `row_<index>`.

## CSV file

**Prerequisites:** a `.csv` file with either one column holding a JSON array,
or one column per dimension.

```yaml
source:
  type: csv_file
  path: ./vectors.csv
  id_column: id                   # default: id
  vector_column: vector           # default: vector
  vector_spread: false            # true = read dim_0, dim_1, ... instead
  dim_prefix: "dim_"              # default: dim_
  delimiter: ","                  # default: ,
  has_header: true                # default: true

destination:
  path: ./velesdb_data
  collection: csv_vectors
  dimension: 4
  metric: cosine

options:
  batch_size: 1000
```

The field names are `id_column` / `vector_column` / `dim_prefix` — **not**
`id_field` / `vector_field` / `vector_columns_prefix`. Because unknown YAML keys
are ignored, the wrong spelling silently falls back to the defaults `id` and
`vector`.

---

## Sparse vector extraction

Sparse vectors (SPLADE / learned-sparse, BM25-style indexes) are carried to the
destination **only** where the source API exposes them:

| Source | Sparse extraction |
|---|---|
| Qdrant (named sparse vectors) | Yes |
| Pinecone (`sparseValues`) | Yes |
| Supabase, Weaviate, Milvus, ChromaDB, Elasticsearch, Redis, JSON, CSV | No |

If your source stores sparse vectors and is not on the "Yes" list, those sparse
vectors are dropped silently; dense vectors and payloads still migrate. Export
them separately and re-ingest, or open an issue asking for connector support.

## Dimension detection

Every connector reports a dimension through `get_schema()`, which
`velesdb-migrate schema` and `velesdb-migrate detect` print:

| Source | Detection method |
|---|---|
| Supabase | fetch 1 row, parse the pgvector wire format |
| Qdrant | collection info API |
| Pinecone | index stats API |
| Weaviate | GraphQL fetch of 1 vector |
| Milvus | schema field type |
| ChromaDB | fetch 1 embedding |
| Elasticsearch | first hit of the index |
| Redis | first document matched by the index |
| JSON / CSV file | length of the first record's vector |

Common embedding dimensions:

| Dimension | Model |
|---|---|
| 384 | sentence-transformers all-MiniLM-L6-v2 |
| 768 | sentence-transformers all-mpnet-base-v2 |
| 1024 | Cohere embed-english-v3.0 |
| 1536 | OpenAI text-embedding-ada-002, text-embedding-3-small |
| 3072 | OpenAI text-embedding-3-large |

## Removed sources

| Source | Removed in | Reason | Workaround |
|---|---|---|---|
| PostgreSQL / pgvector (direct SQL) | v1.13 | The connector was a stub: `get_schema()` and `extract_batch()` returned `UnsupportedSource` at runtime. | Use `supabase` for Supabase projects; for self-hosted pgvector export rows to JSONL and use `json_file`. |
| MongoDB Atlas | v1.13 | Relied on the MongoDB Atlas Data API, deprecated by MongoDB on 2025-09-30. | `mongoexport --collection <c> --out data.jsonl`, then migrate with `json_file`. |

---

`velesdb-migrate v4.3.0` · Last updated: 2026-08-08 · Applies to: velesdb-core 4.3.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
