# velesdb-migrate

> Bulk-loads vectors from Pinecone, Qdrant, Milvus and seven other sources into a local VelesDB database.

[![crates.io](https://img.shields.io/crates/v/velesdb-migrate.svg)](https://crates.io/crates/velesdb-migrate)
[![docs.rs](https://docs.rs/velesdb-migrate/badge.svg)](https://docs.rs/velesdb-migrate)
[![License](https://img.shields.io/badge/license-VelesDB%20Core%201.0-blue.svg)](./LICENSE)

## Objective

Moving an embedding corpus between vector databases normally means writing a
throwaway ETL script: paginate the source API, reshape ids and payloads, batch
the writes, and restart from scratch whenever the run dies halfway through.
`velesdb-migrate` replaces that script with a YAML file. It handles pagination,
id normalisation, payload mapping, dimension checks, retries and
checkpoint/resume, and writes into a VelesDB collection it creates for you.

If you are not moving data *into* VelesDB, this crate has nothing for you.

## Use cases

- Evaluating VelesDB on a copy of a production Pinecone index before committing.
- Leaving a managed vector database for a self-hosted, single-binary deployment.
- Loading an embedding batch produced by an offline pipeline from JSON or CSV.
- Re-importing an export after an incident, resuming an interrupted run.
- Consolidating several Qdrant or Weaviate collections into one VelesDB database.

## Supported sources

| Source | Protocol | Notes |
|---|---|---|
| Supabase | PostgREST | pgvector-enabled projects |
| Qdrant | REST | Scroll pagination, named + sparse vectors |
| Pinecone | REST | Serverless and pod indexes, `sparseValues` |
| Weaviate | GraphQL | Cursor pagination; list the properties to keep |
| Milvus / Zilliz Cloud | REST v2 | |
| ChromaDB | REST | Tenant / database isolation |
| Elasticsearch / OpenSearch | REST | `search_after` pagination |
| Redis Stack | RESP + RediSearch | Requires the default `redis-source` feature |
| JSON file | local file | `.json` array, universal fallback for exports |
| CSV file | local file | one JSON column or one column per dimension |

PostgreSQL/pgvector (direct SQL) and MongoDB Atlas were removed in v1.13; both
have documented workarounds in the
[source reference](../../docs/guides/MIGRATE_SOURCES.md#removed-sources), which
also covers the per-source YAML fields and sparse-vector support.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | Workspace MSRV; only needed to build from source |
| VelesDB destination | — | A writable local directory; created if absent |
| Source database | — | Reachable over the network, with read credentials |
| Disk space | — | Roughly the source corpus size at `storage_mode: full` |

The destination is always a **local** VelesDB data directory. Migrating into a
remote `velesdb-server` over HTTP is not supported.

## Installation

```bash
cargo install velesdb-migrate
```

From a clone of the repository:

```bash
cargo install --path crates/velesdb-migrate
```

Prebuilt binaries for `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`,
`aarch64-apple-darwin` and `x86_64-apple-darwin` ship in the `velesdb-*` archives
attached to each [GitHub release](https://github.com/cyberlife-coder/VelesDB/releases).

## First success in 60 seconds

No external database needed: migrate three vectors from a JSON file.

```bash
mkdir -p /tmp/veles-migrate-demo && cd /tmp/veles-migrate-demo

cat > vectors.json <<'EOF'
[
  {"id": "doc-1", "vector": [0.10, 0.20, 0.30, 0.40], "title": "Onboarding guide"},
  {"id": "doc-2", "vector": [0.90, 0.10, 0.05, 0.00], "title": "Billing FAQ"},
  {"id": "doc-3", "vector": [0.25, 0.25, 0.25, 0.25], "title": "Release notes"}
]
EOF

cat > migration.yaml <<'EOF'
source:
  type: json_file
  path: ./vectors.json
  id_field: id
  vector_field: vector

destination:
  path: ./velesdb_data
  collection: imported_docs
  dimension: 4
  metric: cosine
  storage_mode: full

options:
  batch_size: 1000
EOF

velesdb-migrate run --config migration.yaml
```

Expected output (timestamps, duration, throughput and the SIMD line vary with
the machine):

```text
2026-07-25T16:30:04.286105Z  INFO Loading configuration from "migration.yaml"
2026-07-25T16:30:04.287762Z  INFO Starting migration...
2026-07-25T16:30:04.288537Z  INFO Starting migration pipeline
2026-07-25T16:30:04.289081Z  INFO Source schema: 4 dimension, Some(3) total vectors
2026-07-25T16:30:04.289713Z  INFO SIMD features detected - direct dispatch enabled avx512=false avx2=false
2026-07-25T16:30:04.411457Z  INFO Migration complete: 3 extracted, 3 loaded, 0 failed in 0.12s (24 pts/sec)

✅ Migration Complete!
   Extracted: 3
   Loaded:    3
   Failed:    0
   Duration:  0.12s
   Throughput: 24 vectors/sec
```

Success is `Loaded: 3` with `Failed: 0` and exit code 0. Anything else — a
non-zero `Failed`, or an `Error:` line instead of the summary — means the
migration did not complete; see [Troubleshooting](#troubleshooting).

Verify the result with the VelesDB CLI (`cargo install velesdb-cli`):

```bash
velesdb collection show ./velesdb_data imported_docs
```

```text
Collection Details
  Name: imported_docs
  Type: Vector
  Dimension: 4
  Metric: Cosine
  Point Count: 3
  Storage Mode: Full
  Est. Memory: 0.00 MB
```

## Configuration

The YAML file has four top-level keys — `source`, `destination`, `options` and
the optional `relations`. Full schema, defaults and accepted enum values are in
the [CLI and configuration reference](../../docs/guides/MIGRATE_CLI.md#configuration-schema).

Environment variables read by the tool:

| Variable | Default | Effect |
|---|---|---|
| `VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS` | unset | `1` or `true` lets the six URL-validating connectors reach loopback, RFC 1918 and reserved hostnames. Required for any `http://localhost:...` source. |

Values are **not** interpolated into the YAML: `api_key: ${MY_KEY}` is sent
verbatim as the API key. See
[Secrets](../../docs/guides/MIGRATE_OPERATIONS.md#secrets) for the patterns that
do work.

## Examples

Ten starter configurations live in [`examples/`](./examples), one per source.
Six of them (`chromadb`, `milvus`, `pinecone`, `qdrant`, `supabase`, `weaviate`)
pass `velesdb-migrate validate` unchanged; the `csv`, `elasticsearch`, `json`
and `redis` files omit the required `destination.dimension` and need it added
before use.

`velesdb-migrate init --source <type>` generates a validated config, but only
for `qdrant`, `pinecone`, `weaviate`, `milvus`, `chromadb` and `supabase`.

## API / commands

Six subcommands: `wizard`, `run`, `validate`, `schema`, `init`, `detect`.
`--dry-run`, `--verbose` and `--batch-size` are **global** flags and must come
before the subcommand (`velesdb-migrate --dry-run run --config file.yaml`).
Full command surface: [CLI reference](../../docs/guides/MIGRATE_CLI.md).

Library API (`Pipeline`, `MigrationConfig`, `SourceConnector`, `Transformer`):
[docs.rs/velesdb-migrate](https://docs.rs/velesdb-migrate).

## Known limits

- **Local destination only.** No migration into a remote `velesdb-server`.
- **One-shot copy, not replication.** There is no change-data-capture, no
  incremental sync and no delta detection; a second run re-imports everything
  from offset zero.
- **Dense vectors are mandatory.** Sparse-only points are rejected, and sparse
  vectors are extracted from Qdrant and Pinecone only.
- **No dimension conversion.** Source and destination dimensions must match
  exactly; the run aborts on mismatch.
- **Unknown YAML keys are ignored, not rejected.** A misspelled field silently
  falls back to its default.
- **Retry policy is fixed** (3 attempts, exponential backoff) and cannot be
  tuned from the config file.

## Compatibility

`velesdb-migrate` is a standalone CLI, not an MCP server; there is no agent
client to be compatible with. Supported platforms:

| Platform | Status | Note |
|---|---|---|
| Linux x86_64 (`x86_64-unknown-linux-gnu`) | Supported | Release binary; CI `cargo check --all-features` |
| Windows x86_64 (`x86_64-pc-windows-msvc`) | Supported | Release binary; CI `cargo check --all-features` |
| macOS Apple Silicon (`aarch64-apple-darwin`) | Supported | Release binary; the `native-tls` HTTP backend is chosen for this target |
| macOS Intel (`x86_64-apple-darwin`) | Supported | Release binary |
| Linux ARM64, other targets | Build from source | Not covered by the release matrix |
| Rust toolchain | 1.90+ | Workspace MSRV |
| VelesDB destination format | 4.0.0 | Written through `velesdb-core` 4.0.0 |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Configuration error: URL 'http://localhost:6333' targets reserved hostname 'localhost' ...` | Anti-SSRF policy refuses local hosts | `export VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1` |
| `Schema mismatch: Source dimension 4 != destination dimension 8` | `destination.dimension` differs from the source | Run `velesdb-migrate schema --config <file>` and use the reported dimension |
| `destination: missing field 'dimension' at line N` | `dimension` has no default value | Add `dimension: <N>` under `destination` |
| `error: unexpected argument '--dry-run' found` | Global flag placed after the subcommand | `velesdb-migrate --dry-run run --config <file>` |
| `Unknown source type: json_file` | `init` ships templates for six sources only | Copy a config from the [source reference](../../docs/guides/MIGRATE_SOURCES.md) |

More symptoms, retry behaviour and resume instructions:
[operations guide](../../docs/guides/MIGRATE_OPERATIONS.md#troubleshooting).

## More documentation

- [Source reference](../../docs/guides/MIGRATE_SOURCES.md) — per-source YAML, sparse support, dimension detection, removed connectors.
- [CLI and configuration reference](../../docs/guides/MIGRATE_CLI.md) — commands, flags, full config schema, graph relations, checkpoints.
- [Operations guide](../../docs/guides/MIGRATE_OPERATIONS.md) — throughput tuning, secret handling, network policy, troubleshooting.
- [VelesDB project README](../../README.md) — what you get once the data has landed.

## License

Licensed under the [VelesDB Core License 1.0](./LICENSE) (source-available).
Developed by Julien Lange, WiScale France.

---

`velesdb-migrate v4.1.0` · Last updated: 2026-07-25 · Applies to: velesdb-core 4.1.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
