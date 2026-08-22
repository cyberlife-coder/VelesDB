# velesdb-migrate — operations guide

Throughput tuning, secret handling and troubleshooting for
[`velesdb-migrate`](../../crates/velesdb-migrate/README.md).

Related pages:

- [Source reference](./MIGRATE_SOURCES.md)
- [CLI and configuration reference](./MIGRATE_CLI.md)

---

## Throughput

Migration speed is dominated by the source API, not by VelesDB: the destination
write is a local batch insert, while extraction pays a network round-trip per
batch and is subject to the source's rate limits.

Recommended starting batch sizes, matching the templates the crate ships:

| Source | Recommended `batch_size` |
|---|---|
| Local / self-hosted Qdrant, Weaviate, Milvus, ChromaDB | 1000 |
| Managed Qdrant Cloud, Zilliz, Weaviate Cloud | 500–1000 |
| Supabase (PostgREST row limits) | 500 |
| Pinecone (strict rate limits) | 100 |
| Elasticsearch / OpenSearch, Redis | 100 |
| JSON / CSV file (no network) | 1000 |

The following per-source rates are indicative ranges carried over from earlier
releases. They are **not** benchmarked figures and no CI job asserts them; treat
them as an order of magnitude only.

| Source | Indicative rate |
|---|---|
| Local Qdrant | 10,000+ points/s |
| Cloud Qdrant | 1,000–5,000 points/s |
| Supabase | 1,000–3,000 points/s |
| Pinecone | 500–2,000 points/s |
| Weaviate | 2,000–5,000 points/s |
| Milvus | 3,000–8,000 points/s |
| ChromaDB | 2,000–5,000 points/s |

Tuning checklist:

1. Always start with `velesdb-migrate --dry-run run --config <file>`: it walks
   the whole source and reports the point count without writing.
2. Lower `batch_size` for managed sources; raise it for local ones.
3. Larger batches hold more points in memory at once. Reduce `batch_size`
   before anything else if the process grows too large.
4. `storage_mode: sq8` cuts destination memory ~4x with ~99% recall;
   `binary` cuts it ~32x with ~95% recall.
5. Leave `checkpoint_enabled: true` on large migrations so an interruption
   resumes instead of restarting.

## Secrets

**`velesdb-migrate` does not expand environment variables in YAML.**
`MigrationConfig::from_file` reads the file and hands it straight to
`serde_yaml`, with no interpolation step. A config containing

```yaml
source:
  api_key: ${SUPABASE_SERVICE_KEY}
```

sends the literal string `${SUPABASE_SERVICE_KEY}` as the API key, and the
source rejects it with an authentication error. The `${...}` syntax that appears
in some shipped example files is inert.

Workable patterns:

```bash
# Generate the config at run time from a template, then delete it.
sed "s|__API_KEY__|$SUPABASE_SERVICE_KEY|" migration.template.yaml > migration.yaml
velesdb-migrate run --config migration.yaml
rm -f migration.yaml
```

```bash
# Or keep a config file readable only by its owner.
chmod 600 migration.yaml
```

Recommended credentials per source:

| Source | Recommended credential |
|---|---|
| Supabase | Service-role key, read-only if RLS permits |
| Qdrant | Read-only API key |
| Pinecone | Read-only API key |
| Weaviate | Read-only auth token |
| Elasticsearch | API key scoped to read on the index |
| Redis | User restricted to `FT.SEARCH` on the index |

Keep configs out of version control:

```gitignore
migration.yaml
*.migration.yaml
.env
```

Credentials must never be embedded in the `url`: the SSRF validator rejects any
URL with a `user:pass@host` component.

## Network policy

The URL validator applied by the `qdrant`, `weaviate`, `milvus`,
`elasticsearch`, `redis` and `supabase` connectors enforces:

1. Scheme in `http`, `https`, `redis`, `rediss`, `postgres`, `postgresql`.
2. No userinfo (`user:pass@host`).
3. A non-empty host.
4. No loopback, RFC 1918 private, or link-local address.
5. No reserved hostname (`localhost`, `*.local`, `*.internal`, `*.arpa`), and
   an explicit rejection of the cloud metadata endpoint `169.254.169.254`.

Checks 4 and 5 — and only those two — are bypassed by
`VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1` (or `true`), the local-development
escape hatch. Checks 1 to 3 always apply.

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `Configuration error: URL 'http://localhost:6333' targets reserved hostname 'localhost' ...` | Anti-SSRF policy refuses local hosts | `export VELESDB_MIGRATE_ALLOW_PRIVATE_NETWORKS=1` |
| `Schema mismatch: Source dimension 4 != destination dimension 8` | `destination.dimension` differs from the source | Run `velesdb-migrate schema --config <file>` and copy the reported dimension |
| `destination: missing field 'dimension' at line N` | `dimension` has no default | Add `dimension: <N>` under `destination` |
| `error: unexpected argument '--dry-run' found` | `--dry-run` is a global flag | `velesdb-migrate --dry-run run --config <file>` |
| `Unknown source type: json_file` from `init` | Templates exist for six sources only | Copy a config from [the source reference](./MIGRATE_SOURCES.md) |
| `Configuration error: URL '<url>' must not contain userinfo (user:pass@host)` | Credentials embedded in the URL | Move them to the connector's `api_key` / `username` / `password` fields |
| Migrated points have no payload (Weaviate) | `properties` was empty, so only `_additional { id vector }` was requested | List every property to migrate |
| `Source connection error: ...` | Wrong URL, unreachable host, bad credentials, missing collection | Check the protocol prefix, network reachability, credentials, and that the collection exists |
| Source rate-limit / HTTP 429 errors | Batch too large for a managed source | Lower `batch_size` to 100 or 50 |
| Process memory grows too large | Batch too large, or full-precision storage | Lower `batch_size`; switch `storage_mode` to `sq8` |

### Retries

Each extraction batch is retried up to 3 times with exponential backoff and
jitter (100 ms initial delay, 2x multiplier, 5 s cap). Rate limits, I/O errors,
timeouts, connection resets and 5xx responses are treated as retryable;
authentication and schema errors are not. Retry behaviour is not configurable
from YAML — see
[`velesdb_migrate::retry`](https://docs.rs/velesdb-migrate/latest/velesdb_migrate/retry/index.html).

### Resuming a failed migration

```bash
# Re-run the same command: it resumes from the last checkpoint.
velesdb-migrate run --config migration.yaml

# Or start over by deleting the checkpoint first.
rm -f ./velesdb_data/.velesdb_migrate_checkpoint_qdrant_qdrant_docs.json
velesdb-migrate run --config migration.yaml
```

The checkpoint file name is
`.velesdb_migrate_checkpoint_<source_type>_<destination_collection>.json`,
written inside `destination.path` unless `options.checkpoint_path` overrides it.
A completed migration deletes its own checkpoint.

---

`velesdb-migrate v5.1.0` · Last updated: 2026-08-13 · Applies to: velesdb-core 5.2.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
