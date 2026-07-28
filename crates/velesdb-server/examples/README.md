# velesdb-server examples

Runnable scripts for the HTTP/REST server. Every example is a self-contained
POSIX shell script: it starts its own server on its own port and in its own
temporary data directory, runs the requests, prints the responses, and shuts
the server down again. Nothing is left behind.

## Prerequisites

| Tool | Why |
|---|---|
| `velesdb-server` on `PATH` | `cargo install velesdb-server`, or run `cargo build --release -p velesdb-server` from a clone and use `target/release/velesdb-server` |
| `curl` | every request is a plain `curl` call |
| `python3` *(optional)* | only used to pretty-print JSON; the scripts fall back to raw output when it is missing |

The scripts pick up the binary from the `VELESDB_SERVER_BIN` environment
variable when it is set, so a source build works without installing:

```bash
cd /path/to/velesdb
cargo build --release -p velesdb-server
VELESDB_SERVER_BIN=$PWD/target/release/velesdb-server \
  ./crates/velesdb-server/examples/01_quickstart.sh
```

## Index

| Example | What it shows | Endpoints exercised |
|---|---|---|
| [`01_quickstart.sh`](./01_quickstart.sh) | The README "first success in 60 seconds", end to end: create a collection, upsert three points, search the nearest two. | `GET /v1/ready`, `POST /v1/collections`, `POST /v1/collections/{name}/points`, `POST /v1/collections/{name}/search` |
| [`02_text_and_hybrid_search.sh`](./02_text_and_hybrid_search.sh) | The three retrieval modes on the same corpus: dense vector, BM25 text, and weighted hybrid. | `POST /v1/collections/{name}/search`, `.../search/text`, `.../search/hybrid` |
| [`03_velesql.sh`](./03_velesql.sh) | VelesQL over HTTP: a projected `SELECT`, a filtered `SELECT`, and the `EXPLAIN` plan for the same statement. | `POST /v1/query`, `POST /v1/query/explain` |
| [`04_auth_and_rate_limit.sh`](./04_auth_and_rate_limit.sh) | Bearer API keys and the per-IP rate limiter, including the rejections they produce (`401`, `429`) and the probes that stay open. | `GET /v1/health`, `GET /v1/ready`, `POST /v1/collections`, `GET /metrics` |
| [`velesdb.toml`](./velesdb.toml) | The configuration file the crate README shows, annotated section by section: `[server]`, `[auth]`, `[tls]`, `[cors]`. It is the reference for a real deployment — the scripts themselves use CLI flags and environment variables so that they need no absolute paths. | — |

## Conventions used by every script

- `set -euo pipefail` — the script stops at the first failed request instead of
  printing a misleading success.
- Each script binds a distinct port (`8081`–`8084`) so two of them can run at
  the same time, and so neither collides with a server you already have on the
  default `8080`.
- The data directory is a `mktemp -d` path, removed by an `EXIT` trap along
  with the server process. Stopping the server with `SIGTERM` (what `kill`
  sends by default) drains in-flight requests and flushes the write-ahead logs
  before the process exits.
- Responses are printed verbatim. `id` values come back as **JSON strings**
  (`"id":"1"`), not numbers: point ids are `u64` and would lose precision above
  2^53 in a JavaScript client.

## Going further

- [REST tour](../../../docs/guides/SERVER_REST_TOUR.md) — every endpoint family with `curl` recipes.
- [Configuration](../../../docs/guides/CONFIGURATION.md) — every TOML key and environment variable.
- [Deployment](../../../docs/guides/SERVER_DEPLOYMENT.md) — Docker, Kubernetes probes, CORS.
- [Server security](../../../docs/guides/SERVER_SECURITY.md) — API keys, rotation, TLS.
- [Error codes](../../../docs/reference/ERROR_CODES.md) — the `VELES-NNN` catalogue.
