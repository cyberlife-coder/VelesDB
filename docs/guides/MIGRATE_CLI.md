# velesdb-migrate — CLI and configuration reference

Command surface and full YAML schema for
[`velesdb-migrate`](../../crates/velesdb-migrate/README.md).

Related pages:

- [Source reference](./MIGRATE_SOURCES.md)
- [Operations: performance, secrets, troubleshooting](./MIGRATE_OPERATIONS.md)

---

## Flag placement matters

`--dry-run`, `--verbose`, `--batch-size` and the bare `--config` are **global**
options declared on the root command, not on `run`. They must appear *before*
the subcommand:

```bash
# Correct
velesdb-migrate --dry-run run --config migration.yaml

# Rejected: error: unexpected argument '--dry-run' found
velesdb-migrate run --config migration.yaml --dry-run
```

`velesdb-migrate --config migration.yaml` with no subcommand runs the migration
directly, so `--config ... --dry-run` also works in that form.

## Commands

| Command | Purpose | Required arguments |
|---|---|---|
| `wizard` | Interactive migration wizard | none |
| `run` | Run a migration from a config file | `-c, --config <FILE>` |
| `validate` | Parse and validate a config file (no network) | `-c, --config <FILE>` |
| `schema` | Connect to the source and print its schema | `-c, --config <FILE>` |
| `init` | Write a starter config for a source type | `-s, --source <TYPE>` |
| `detect` | Connect, introspect, and generate a config | `-s`, `-u`, `-n` |

Global options:

| Option | Effect |
|---|---|
| `-c, --config <FILE>` | Config file; runs the migration when no subcommand is given |
| `--dry-run` | Extract and transform, never write to the destination |
| `-v, --verbose` | Raise the log level from `INFO` to `DEBUG` |
| `--batch-size <N>` | Override `options.batch_size` from the config |
| `-h, --help` / `-V, --version` | Help / version (`velesdb-migrate 4.0.0`) |

Signatures are generated on
[docs.rs/velesdb-migrate](https://docs.rs/velesdb-migrate); this page documents
only the CLI surface.

### `wizard`

```bash
velesdb-migrate wizard
```

The wizard asks for the source type, the connection details and the
destination, connects to introspect the collection, and runs the migration
after a confirmation prompt — no config file involved. Its source menu covers
all ten connectors, including the four `init` has no template for.

```text
╔═══════════════════════════════════════════════════════════════╗
║         🚀 VELESDB MIGRATION WIZARD                           ║
║         Migrate your vectors in under 60 seconds              ║
╚═══════════════════════════════════════════════════════════════╝

? Where are your vectors stored?
  ❯ Supabase (PostgreSQL + pgvector)
    Qdrant
    Pinecone
    Weaviate
    Milvus / Zilliz Cloud
    ChromaDB
    JSON File (local import)
    CSV File (local import)
    Elasticsearch / OpenSearch
    Redis Vector Search
```

### `init`

```bash
velesdb-migrate init --source qdrant --output qdrant.yaml
```

Templates exist for six source types only: `qdrant`, `pinecone`, `weaviate`,
`milvus`, `chromadb`, `supabase`. Any other value — including `elasticsearch`,
`redis`, `json_file` and `csv_file`, which the command's own help text lists —
prints `Unknown source type: <x>` and exits. For those four, copy a config from
[the source reference](./MIGRATE_SOURCES.md) instead.

The six generated templates all pass `velesdb-migrate validate` unchanged.

### `detect`

```bash
velesdb-migrate detect \
  --source qdrant \
  --url https://xyz.aws.cloud.qdrant.io \
  --collection my_vectors \
  --api-key "$QDRANT_API_KEY" \
  --output migration.yaml
```

| Option | Default | Effect |
|---|---|---|
| `-s, --source <TYPE>` | required | `supabase`, `qdrant`, `pinecone`, `weaviate`, `milvus`, `chromadb`, `json_file`/`json`, `csv_file`/`csv`, `elasticsearch`, `redis` |
| `-u, --url <URL>` | required | Source URL |
| `-n, --collection <NAME>` | required | Collection / table / index name |
| `-a, --api-key <KEY>` | none | API key when the source needs one |
| `-o, --output <FILE>` | `migration.yaml` | Generated config path |
| `--dest-path <PATH>` | `./velesdb_data` | VelesDB destination directory |

`detect` connects, fetches the schema, prints a summary, and writes the config:

```text
🔍 Auto-detecting schema from supabase source...
   URL: https://your-project.supabase.co
   Collection: documents

🔌 Connecting to source...
📊 Fetching schema...

✅ Schema Detected!
┌─────────────────────────────────────────────
│ Source Type:  supabase
│ Collection:   documents
│ Dimension:    1536
│ Total Count:  14053 vectors
├─────────────────────────────────────────────
│ Detected Metadata Fields:
│   • title (string)
│   • content (string)
│   • created_at (string)
└─────────────────────────────────────────────

📝 Configuration generated: "migration.yaml"

💡 Next steps:
   1. Review and edit the config file
   2. Verify column names (vector_column, id_column, payload_columns)
   3. Run: velesdb-migrate run --config "migration.yaml" --dry-run
   4. Run: velesdb-migrate run --config "migration.yaml"
```

Ignore step 3 as printed: `--dry-run` must precede the subcommand
(`velesdb-migrate --dry-run run --config migration.yaml`).

Review the generated `vector_column`, `id_column` and `payload_columns` before
running the migration — the detector infers them from a sample. The dimension it
reports comes from the source itself (1536 for OpenAI `text-embedding-ada-002`,
768 for `all-mpnet-base-v2`, and so on), so it does not need to be guessed.

### `schema`

```bash
velesdb-migrate schema --config migration.yaml
```

Prints the source type, collection, dimension, total count and detected fields.
Use it whenever a dimension mismatch aborts a run.

## Recommended workflows

**Auto-detect (fastest):**

1. `velesdb-migrate detect --source <type> --url <url> --collection <name>`
2. Review `migration.yaml`
3. `velesdb-migrate --dry-run run --config migration.yaml`
4. `velesdb-migrate run --config migration.yaml`
5. `velesdb collection show ./velesdb_data <collection>`

**Manual:**

1. `velesdb-migrate init --source <type> --output migration.yaml`
2. Edit credentials, dimension and collection names
3. `velesdb-migrate validate --config migration.yaml`
4. `velesdb-migrate schema --config migration.yaml`
5. `velesdb-migrate --dry-run run --config migration.yaml`
6. `velesdb-migrate run --config migration.yaml`
7. `velesdb collection show ./velesdb_data <collection>`

`--dry-run` extracts and transforms every point but loads none, so the summary
reports `Loaded: 0` with a non-zero `Extracted`.

## Configuration schema

A config file has four top-level keys: `source` (see
[the source reference](./MIGRATE_SOURCES.md)), `destination`, `options` and the
optional `relations`.

Unknown keys are ignored rather than rejected. In particular
`destination.type` and `options.storage` — which appear in some shipped example
files — are **not** part of the schema and have no effect.

### `destination`

| Key | Type | Default | Effect |
|---|---|---|---|
| `path` | string | required | VelesDB data directory (created if absent) |
| `collection` | string | required | Destination collection, created if absent |
| `dimension` | integer | required | Vector dimension; must equal the source dimension |
| `metric` | enum | `cosine` | `cosine`, `euclidean`, `dot` (aliases `dot_product`, `DotProduct`), `hamming`, `jaccard` |
| `storage_mode` | enum | `full` | `full`, `sq8` (4x), `binary` (32x), `pq` (alias `product_quantization`), `rabitq` |
| `graph_collection` | string | none | Graph collection receiving the edges declared in `relations` |

`dimension` has no default: omitting it fails with
`destination: missing field 'dimension'`.

### `options`

| Key | Type | Default | Effect |
|---|---|---|---|
| `batch_size` | integer | `1000` | Points extracted and written per batch; must be > 0 |
| `workers` | integer | `4` | Parallel point-preparation workers before the batch write; must be > 0 |
| `checkpoint_enabled` | boolean | `true` | Save a resume point after each successful batch |
| `checkpoint_path` | string | auto | Override the checkpoint file location |
| `dry_run` | boolean | `false` | Same as the `--dry-run` flag |
| `continue_on_error` | boolean | `false` | Skip failed points instead of aborting |
| `field_mappings` | map | `{}` | Rename payload fields during transformation |
| `allow_metric_mismatch` | boolean | `false` | Proceed when the source reports a different distance metric than the destination |

Renaming payload fields:

```yaml
options:
  field_mappings:
    legacy_title: title
    doc_content: content
    created: created_at
```

### `relations` (graph edges)

When the source payload carries foreign keys, they can be materialised as graph
edges in a VelesDB `GraphCollection`. `destination.graph_collection` must be set
or the graph phase fails with `graph_collection not configured`; an empty
`relations` list skips the phase entirely.

```yaml
destination:
  path: ./velesdb_data
  collection: articles
  dimension: 768
  graph_collection: article_graph

relations:
  - from_column: author_id
    to_table: authors
    to_column: id          # default: id
    edge_label: AUTHORED_BY
    weight_column: score   # optional numeric edge weight
```

## Checkpoint and resume

With `checkpoint_enabled: true` (the default, and disabled automatically for
dry runs), the pipeline writes a checkpoint after each successfully flushed
batch. The default path is inside the destination directory:

```text
<destination.path>/.velesdb_migrate_checkpoint_<source_type>_<collection>.json
```

Re-running the same command resumes from the last saved offset and logs
`Resuming migration from checkpoint at '<path>'`. The file is deleted once the
migration completes, so a successful run leaves no checkpoint behind. To start
over after a partial failure, delete the file (or point `checkpoint_path`
elsewhere) before re-running.

---

`velesdb-migrate v4.1.0` · Last updated: 2026-07-25 · Applies to: velesdb-core 4.3.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
