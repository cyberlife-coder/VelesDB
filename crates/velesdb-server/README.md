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

## Operations

Everything an operator configures — API keys and their rotation, TLS, the
graceful-shutdown sequence and its WAL flush guarantee, the `/health` and
`/ready` probes — lives in **[Server security](../../docs/guides/SERVER_SECURITY.md)**,
which is the canonical reference and covers each of them in more depth than a
README should.

Docker, Kubernetes manifests, rate limiting, CORS and the startup update check
are in **[Deployment](../../docs/guides/SERVER_DEPLOYMENT.md)**.

The short version:

| Concern | Set | Default |
|---|---|---|
| API keys | `VELESDB_API_KEYS`, or `api_keys` in `velesdb.toml` | none — the server runs in local dev mode and accepts every request |
| TLS | `VELESDB_TLS_CERT` / `VELESDB_TLS_KEY`, or `--tls-cert` / `--tls-key` | off (plain HTTP) |
| Data directory | `VELESDB_DATA_DIR` | `./data` |
| Bind address | `VELESDB_HOST` / `VELESDB_PORT` | `127.0.0.1:8080` |
| Config file | `VELESDB_CONFIG` or `--config` | `./velesdb.toml` if present |

Configuration priority, highest first: **CLI flags > environment variables >
`velesdb.toml` > built-in defaults**. Every section of the file is optional;
declare only what you override.

Distance metrics accepted by the API (`cosine`, `euclidean`, `dot` — aliases
`dotproduct`, `inner`, `ip` —, `hamming`, `jaccard`) are listed with their use
cases in the [REST tour](../../docs/guides/SERVER_REST_TOUR.md); measured
latency figures live in the [benchmarks](../../docs/BENCHMARKS.md), pinned to
[`promise-contract.json`](../../docs/reference/promise-contract.json).

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
- **Swagger UI is opt-in at build time** (`--features swagger-ui`); the released default build does not serve `/swagger-ui` or `/api-docs/openapi.json`. Building with that feature needs network access: `utoipa-swagger-ui`'s build script downloads the Swagger UI distribution, so it fails in air-gapped builds. Offline, skip the feature and point any OpenAPI viewer at [docs/openapi.json](../../docs/openapi.json) instead.
- **The `/v1` prefix is added by the binary.** Embedding `velesdb_server::routes::api_routes()` in your own Axum application gives you the unversioned routes; nest them yourself if you want the versioned form.
- Engine-level limits (query length caps, GROUP BY ceilings, scan caps) are listed in [docs/reference/KNOWN_LIMITATIONS.md](../../docs/reference/KNOWN_LIMITATIONS.md).

## Compatibility

| Environment | Status | Note |
|---|---|---|
| Linux x86_64 (glibc) | Supported | `.tar.gz` and `.deb` release artifacts |
| macOS aarch64 (Apple Silicon) | Supported | `.tar.gz` release artifact |
| macOS x86_64 (Intel) | Supported | `.tar.gz` release artifact |
| Windows x86_64 (MSVC) | Supported | Portable `.zip`; no signed MSI installer yet |
| Docker | Supported | Repository `Dockerfile`: `rust:1.98-bookworm` builder, `debian:bookworm-slim` runtime, non-root user, port 8080 |
| Rust toolchain | 1.90 or later | Workspace MSRV, for `cargo install` and source builds |
| `velesdb-core` | 5.0.0 | Same workspace version; non-optional dependency with `openapi` + `persistence` enabled |
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

`velesdb-server v6.0.0` · Last updated: 2026-09-02 · Applies to: velesdb-core 6.0.0 · [Report a docs error](https://github.com/cyberlife-coder/VelesDB/issues)
