# velesdb-server

> HTTP/REST server that exposes a VelesDB database — vector, text, graph and VelesQL — to any language.

[![crates.io](https://img.shields.io/crates/v/velesdb-server.svg)](https://crates.io/crates/velesdb-server)
[![docs.rs](https://docs.rs/velesdb-server/badge.svg)](https://docs.rs/velesdb-server)
[![License](https://img.shields.io/badge/license-VelesDB_Core_1.0-blue.svg)](../../LICENSE)

## Objective

`velesdb-core` is an embedded engine: it lives inside one Rust process. As soon
as a second process — a Python service, a Node worker, an agent running on
another machine — needs the same data, you need a network boundary.
`velesdb-server` is that boundary: a single self-contained binary that wraps the
engine in an Axum HTTP API, adds API keys, TLS, rate limiting, health probes and
Prometheus metrics, and persists everything to a data directory (WAL + mmap)
that survives restarts. No JVM, no sidecar, no external dependency.

> The engine behind **VelesDB — the explainable, local-first memory engine for AI agents.** It fuses vector + graph + columnar under VelesQL; the [`why()`](../velesdb-memory/README.md) recall trail returns the evidence path behind every answer.

## Use cases

- A Python or TypeScript service needs vector search, and you do not want to embed a Rust library in every runtime you ship.
- Several agents on a LAN share one memory store, authenticated with Bearer API keys over TLS.
- A local RAG prototype needs a backend that survives a laptop reboot without setting up a database cluster.
- A Kubernetes deployment needs liveness/readiness probes, `/metrics`, and a clean `SIGTERM` that flushes the write-ahead logs.
- A knowledge graph is queried with Cypher-style `MATCH` and vector similarity in the same request.

## Prerequisites

| Requirement | Minimum version | Note |
|---|---|---|
| Rust | 1.90 | Only to build or `cargo install`; the release archives ship a prebuilt binary. Workspace `rust-version`. |
| C toolchain | any | Needed when building from source: `rustls` uses the `ring` backend. |
| OS | Linux, macOS, Windows | Prebuilt binaries for Linux x86_64, macOS x86_64/aarch64, Windows x86_64. |
| HTTP client | any | `curl` is used in every example below. |

## Installation

```bash
cargo install velesdb-server
```

Prebuilt archives (`.tar.gz`, `.deb`, `.zip`), Docker, and platform-specific
notes: [docs/guides/INSTALLATION.md](../../docs/guides/INSTALLATION.md).
Container and orchestrator setup:
[docs/guides/SERVER_DEPLOYMENT.md](../../docs/guides/SERVER_DEPLOYMENT.md).

Building from a clone of this repository:

```bash
cargo build --release -p velesdb-server
```

## First success in 60 seconds

```bash
# 1. Start the server and wait until it reports readiness
velesdb-server --port 8080 --data-dir ./velesdb_data &
until curl -sf http://localhost:8080/v1/ready > /dev/null; do sleep 1; done

# 2. Create a 4-dimensional collection
curl -sS -X POST http://localhost:8080/v1/collections \
  -H "Content-Type: application/json" \
  -d '{"name": "quickstart", "dimension": 4, "metric": "cosine"}'
echo

# 3. Insert three points
curl -sS -X POST http://localhost:8080/v1/collections/quickstart/points \
  -H "Content-Type: application/json" \
  -d '{"points": [
        {"id": 1, "vector": [1.0, 0.0, 0.0, 0.0], "payload": {"title": "first"}},
        {"id": 2, "vector": [0.9, 0.4, 0.0, 0.0], "payload": {"title": "second"}},
        {"id": 3, "vector": [0.1, 0.9, 0.0, 0.0], "payload": {"title": "third"}}
      ]}'
echo

# 4. Search for the nearest two vectors
curl -sS -X POST http://localhost:8080/v1/collections/quickstart/search \
  -H "Content-Type: application/json" \
  -d '{"vector": [1.0, 0.0, 0.0, 0.0], "top_k": 2}'
echo
```

Expected output — three JSON lines, in this order:

```json
{"message":"Collection created","name":"quickstart","type":"vector","warnings":["Collection dimension and metric are immutable after creation. If your embedding model changes, create a new collection and reindex data.","For first queries, start without strict filters/thresholds, then tighten progressively."]}
{"count":3,"message":"Points upserted"}
{"results":[{"id":"1","score":1.0,"payload":{"title":"first"}},{"id":"2","score":0.91381156,"payload":{"title":"second"}}]}
```

The `code` field is optional and omitted when no structured code applies. Use it for
programmatic error handling (e.g., retry on `VELES-006`, display user hint on `VELES-004`).
See [ERROR_CODES.md](../../docs/reference/ERROR_CODES.md) for the full list.

## Authentication

VelesDB supports optional API key authentication. When no keys are configured, the server runs in **local dev mode** (all requests are accepted). When one or more keys are configured, every request must include a valid `Authorization: Bearer <key>` header.

### Enabling Authentication

**Via environment variable** (comma-separated list):

```bash
VELESDB_API_KEYS="sk-prod-abc123,sk-prod-def456" velesdb-server
```

**Via `velesdb.toml`**:

```toml
[auth]
api_keys = ["sk-prod-abc123", "sk-prod-def456"]
```

You can configure as many keys as you need. This is useful for rotating keys without downtime: add the new key, deploy, then remove the old key.

### Making Authenticated Requests

```bash
curl -X GET http://localhost:8080/collections \
  -H "Authorization: Bearer sk-prod-abc123"
```

If authentication is enabled and the header is missing or invalid, the server returns `401 Unauthorized`:

```json
{"error": "Unauthorized", "message": "missing Authorization header"}
```

### Public Endpoints (No Auth Required)

The following endpoints bypass authentication even when API keys are configured:

| Endpoint | Purpose |
|----------|---------|
| `GET /health`, `GET /v1/health` | Liveness probe |
| `GET /ready`, `GET /v1/ready` | Readiness probe |

This allows load balancers and orchestrators to probe the server without
credentials. `GET /metrics` (Prometheus) is **gated by the API key** when
auth is enabled — see `src/auth.rs::is_public_path` and finding F-02 in
the auth audit; the metrics endpoint can leak collection names and
write rates, so it is intentionally not in the public list.

## TLS / HTTPS

VelesDB supports native TLS via rustls (no OpenSSL dependency). When both a certificate and private key are provided, the server binds with HTTPS instead of HTTP.

### Generating Self-Signed Certificates (Development)

```bash
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem \
  -days 365 -nodes -subj "/CN=localhost"
```

### Configuring TLS

**Via environment variables**:

```bash
VELESDB_TLS_CERT=./cert.pem VELESDB_TLS_KEY=./key.pem velesdb-server
```

**Via CLI flags**:

```bash
velesdb-server --tls-cert ./cert.pem --tls-key ./key.pem
```

**Via `velesdb.toml`**:

```toml
[tls]
cert = "/etc/velesdb/cert.pem"
key  = "/etc/velesdb/key.pem"
```

Both `cert` and `key` must be provided together. The server will refuse to start if only one is set or if the files do not exist.

### Making Requests Over HTTPS

With a self-signed certificate:

```bash
curl --cacert cert.pem https://localhost:8080/health
```

Or skip verification during development (not for production):

```bash
curl -k https://localhost:8080/health
```

## Graceful Shutdown

VelesDB performs a clean shutdown when it receives **SIGINT** (Ctrl+C) or **SIGTERM** (on Unix). The shutdown sequence:

1. **Stop accepting new connections** -- the listening socket is closed immediately.
2. **Drain in-flight requests** -- the server waits up to `shutdown_timeout_secs` (default: 30 seconds) for active connections to complete.
3. **Flush all WALs** -- every collection's Write-Ahead Log is flushed to disk, ensuring no acknowledged writes are lost.
4. **Exit** -- the process terminates cleanly.

If the drain timeout expires with connections still active, the server logs a warning and proceeds to the WAL flush.

### Configuring the Drain Timeout

**Via `velesdb.toml`**:

```toml
[server]
shutdown_timeout_secs = 60
```

The default is 30 seconds, which is sufficient for most workloads.

## Health & Readiness Probes

### `GET /health` -- Liveness Probe

Always returns `200 OK` as long as the process is running. Use this for container liveness checks.

```bash
curl http://localhost:8080/health
```

Response:

```json
{"status": "ok", "version": "4.1.0"}
```

### `GET /ready` -- Readiness Probe

Returns `200 OK` once the database has finished loading all collections from disk. Returns `503 Service Unavailable` while loading.

```bash
curl http://localhost:8080/ready
```

Response (ready):

```json
{"status": "ready", "version": "4.1.0"}
```

Response (not ready):

```json
{"status": "not_ready", "version": "4.1.0"}
```

### Kubernetes Example

```yaml
livenessProbe:
  httpGet:
    path: /health
    port: 8080
  initialDelaySeconds: 5
  periodSeconds: 10

readinessProbe:
  httpGet:
    path: /ready
    port: 8080
  initialDelaySeconds: 2
  periodSeconds: 5
```

## Distance Metrics

| Metric | API Value | Aliases | Use Case |
|--------|-----------|---------|----------|
| Cosine | `cosine` | | Text embeddings |
| Euclidean | `euclidean` | | Spatial data |
| Dot Product | `dot` | `dotproduct`, `inner`, `ip` | Pre-normalized vectors |
| Hamming | `hamming` | | Binary vectors |
| Jaccard | `jaccard` | | Set similarity |

## Performance

Numbers match the canonical contract in
[`docs/reference/promise-contract.json`](../../docs/reference/promise-contract.json)
(i9-14900KF, AVX2, `--release`, `target-cpu=native`):

- **Cosine similarity**: ~33 ns per operation (768D)
- **Dot product**: ~21.7 ns per operation (768D), ~35 Gelem/s
- **HNSW search (index-only)**: ~55 µs (10K vectors, 768D, Balanced mode, k=10)
- **End-to-end search p50**: ~450 µs (10K/384D, WAL ON, recall ≥ 96%)

## Configuration Reference

VelesDB loads configuration with the following priority (highest wins):

**CLI flags > Environment variables > `velesdb.toml` file > Built-in defaults**

A custom config file path can be specified with `--config /path/to/velesdb.toml` or `VELESDB_CONFIG`.

### Environment Variables and CLI Flags

| Environment Variable | CLI Flag | Default | Description |
|---------------------|----------|---------|-------------|
| `VELESDB_HOST` | `--host` | `127.0.0.1` | Bind address. Use `0.0.0.0` for network access. |
| `VELESDB_PORT` | `--port` / `-p` | `8080` | Server port. |
| `VELESDB_DATA_DIR` | `--data-dir` / `-d` | `./velesdb_data` | Data directory for persistent storage. |
| `VELESDB_CONFIG` | `--config` / `-c` | `./velesdb.toml` | Path to TOML configuration file (optional). |
| `VELESDB_API_KEYS` | -- | *(empty)* | Comma-separated API keys. When set, enables Bearer token auth. |
| `VELESDB_TLS_CERT` | `--tls-cert` | *(none)* | Path to TLS certificate file (PEM). Requires `VELESDB_TLS_KEY`. |
| `VELESDB_TLS_KEY` | `--tls-key` | *(none)* | Path to TLS private key file (PEM). Requires `VELESDB_TLS_CERT`. |
| `RUST_LOG` | -- | `info` | Log level filter (e.g. `warn`, `info`, `debug`, `trace`). |
| `VELESDB_NO_UPDATE_CHECK` | -- | *(unset)* | Set to `1` to disable startup update check. |

### TOML Configuration File

```toml
[server]
host = "0.0.0.0"
port = 8080
data_dir = "/var/lib/velesdb"
shutdown_timeout_secs = 30

[auth]
api_keys = ["sk-prod-abc123", "sk-prod-def456"]

[tls]
cert = "/etc/velesdb/cert.pem"
key  = "/etc/velesdb/key.pem"
```

All sections are optional; declare only what you override.

## Examples

- [REST tour](../../docs/guides/SERVER_REST_TOUR.md) — every endpoint family with runnable `curl` recipes: collections, quantization, points, search modes, sparse and hybrid search, VelesQL, graph, `MATCH`, indexes, errors.
- [Deployment](../../docs/guides/SERVER_DEPLOYMENT.md) — Docker, Kubernetes probes, rate limiting, CORS, update check.
- [Server security](../../docs/guides/SERVER_SECURITY.md) — API keys, key rotation, TLS, graceful shutdown, health endpoints.
- [Getting started](../../docs/getting-started.md) — the wider VelesDB tour, engine included.

## API / commands

| Surface | Where |
|---|---|
| HTTP endpoint specification | [docs/reference/api-reference.md](../../docs/reference/api-reference.md) |
| Machine-readable schema | [docs/openapi.yaml](../../docs/openapi.yaml), [docs/openapi.json](../../docs/openapi.json) |
| Swagger UI | `http://localhost:8080/swagger-ui` — requires a build with `--features swagger-ui` |
| Rust items (`routes::api_routes`, `config`, `auth`, `tls`) | [docs.rs/velesdb-server](https://docs.rs/velesdb-server) |
| CLI flags | `velesdb-server --help` |
| Error codes | [docs/reference/ERROR_CODES.md](../../docs/reference/ERROR_CODES.md) |

Routes are served under two prefixes: `/v1/…` is canonical, and the
unversioned `/…` form is kept for backward compatibility — its responses carry
`deprecation: true` and `x-api-deprecated: Use /v1/ prefix`.

## Known limits

- **One process per data directory.** The engine takes an exclusive OS-level lock on `<data_dir>/velesdb.lock`. There is no built-in clustering, replication, or sharding: scale vertically, or shard at the application level. See [CONCURRENCY_LOCKING.md](../../docs/guides/CONCURRENCY_LOCKING.md).
- **Authentication is a flat list of API keys.** No users, no roles, no per-collection scoping. Keys are read at startup, so rotation requires a restart (both old and new key can be active during the transition).
- **Rate limiting is per process and in memory.** Replicas do not share a budget; put a shared limiter in front if you need a global one.
- **CORS is permissive by default** (`allowed_origins = ["*"]`); the server warns about it at startup. Restrict `[cors]` before exposing a browser-facing deployment.
- **Swagger UI is opt-in at build time** (`--features swagger-ui`); the released default build does not serve `/swagger-ui` or `/api-docs/openapi.json`.
- **The `/v1` prefix is added by the binary.** Embedding `velesdb_server::routes::api_routes()` in your own Axum application gives you the unversioned routes; nest them yourself if you want the versioned form.
- Engine-level limits (query length caps, GROUP BY ceilings, scan caps) are listed in [docs/reference/KNOWN_LIMITATIONS.md](../../docs/reference/KNOWN_LIMITATIONS.md).

## Compatibility

| Environment | Status | Note |
|---|---|---|
| Linux x86_64 (glibc) | Supported | `.tar.gz` and `.deb` release artifacts |
| macOS aarch64 (Apple Silicon) | Supported | `.tar.gz` release artifact |
| macOS x86_64 (Intel) | Supported | `.tar.gz` release artifact |
| Windows x86_64 (MSVC) | Supported | Portable `.zip`; no signed MSI installer yet |
| Docker | Supported | Repository `Dockerfile`: `rust:1.97-bookworm` builder, `debian:bookworm-slim` runtime, non-root user, port 8080 |
| Rust toolchain | 1.90 or later | Workspace MSRV, for `cargo install` and source builds |
| `velesdb-core` | 4.0.0 | Same workspace version; non-optional dependency with `openapi` + `persistence` enabled |
| HTTP clients | Any | Plain JSON over HTTP/1.1, described by an OpenAPI 3.0 document |

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `curl: (7) Failed to connect to localhost port 8080` | The server is not running, or it bound another address — the startup log prints `Bind address: <host>:<port>`. | Start it, or set `--host` / `--port`. `127.0.0.1` is the default and is not reachable from another machine. |
| `{"error":"[VELES-002] Collection 'x' not found","code":"VELES-002"}` | Wrong collection name, or the server was started on a different `--data-dir` (a missing directory is created empty). | `curl http://localhost:8080/v1/collections` to list what this instance actually holds. |
| `{"error":"Vector dimension mismatch for collection 'demo': expected 4, got 2. …","code":"VELES-004"}` | The query vector does not match the collection dimension; dimension and metric are immutable after creation. | Use the embedding model the collection was built with, or create a new collection and reindex. |
| `401` with `{"error":"Unauthorized","message":"missing Authorization header"}` | API keys are configured, so every route except the health/readiness probes requires a key. | Add `-H "Authorization: Bearer <key>"`. `/metrics` needs it too. |
| `429 Too Many Requests` | The per-IP limiter (100 req/s by default) is saturated; the response carries `retry-after`. | Back off, raise `--rate-limit`, or pass `--rate-limit 0` to disable it. |
| The process exits at startup with `tls_cert is set but tls_key is missing` | TLS needs both files; a half-configured pair is refused rather than silently downgraded to HTTP. | Provide `--tls-cert` and `--tls-key` together, and check both paths exist. |

## License

VelesDB Core License 1.0 — see [LICENSE](../../LICENSE).

---

`velesdb-server v4.1.0` · Last updated: 2026-07-25 · Applies to: velesdb-core 4.1.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
