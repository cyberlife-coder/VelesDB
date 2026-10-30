# VelesDB Server — Deployment

How to run `velesdb-server` outside a developer laptop: containers,
orchestrators, probes, rate limiting, CORS, and runtime sizing.

> **See also:** [SERVER_SECURITY.md](SERVER_SECURITY.md) for API keys, TLS and
> graceful shutdown; [CONFIGURATION.md](CONFIGURATION.md) for the exhaustive
> option reference; [INSTALLATION.md](INSTALLATION.md) for every install path.

---

## 1. Docker

The repository ships a multi-stage `Dockerfile` and a `docker-compose.yml` at
its root.

```bash
# Build from the repository root
docker build -t velesdb .

# Run with a named volume for the data directory
docker run -d --name velesdb -p 8080:8080 -v velesdb_data:/data velesdb
```

The image sets these defaults (see the `Dockerfile`):

| Variable | Value in the image |
|----------|--------------------|
| `VELESDB_DATA_DIR` | `/data` |
| `VELESDB_HOST` | `0.0.0.0` |
| `VELESDB_PORT` | `8080` |

The image sets no `RUST_LOG`, so the log level comes from
`VELESDB_LOGGING_LEVEL` or a mounted `velesdb.toml`'s `[logging] level`, and
defaults to `info` plus `tower_http`'s per-request debug lines. Setting
`RUST_LOG` (e.g. `-e RUST_LOG=info,velesdb_core=debug`) overrides both. Log
format and destination are `[logging] format` / `file` — see
[CONFIGURATION.md](CONFIGURATION.md#section-logging).

It runs as the non-root `velesdb` user, exposes port 8080, and declares a
`HEALTHCHECK` that curls `/health` every 30 s.

> **`VELESDB_HOST=0.0.0.0` binds a publicly reachable address.** With no API
> key configured the server logs a warning at startup and every endpoint —
> including `/metrics` and all data — is reachable by anyone who can reach the
> container. Set `VELESDB_API_KEYS` before exposing it; see
> [SERVER_SECURITY.md](SERVER_SECURITY.md).

> **Bound to `0.0.0.0`, list the names clients use.** The server answers only
> requests whose `Host` is `localhost`, a `127.0.0.0/8` address, `[::1]` or an
> entry of `allowed_hosts`; anything else gets `421` (DNS-rebinding
> protection). `curl localhost:8080` from the Docker host and the image's
> `HEALTHCHECK` work as they are; a client using a DNS name or a LAN address
> needs it listed: `-e VELESDB_ALLOWED_HOSTS=velesdb.example.com,192.168.1.20`.
> `/health` and `/ready` answer any `Host`, so Kubernetes probes need nothing.

### Behind a reverse proxy

Two settings follow from where the browser and the proxy stand:

- **`Host`:** forward the client's original `Host` (nginx:
  `proxy_set_header Host $host;`, which Traefik and Caddy do by default) and
  list the public name in `allowed_hosts`. `X-Forwarded-Host` is never read,
  so a proxy that rewrites `Host` to its upstream name must have *that* name
  listed instead.
- **`Origin`:** a browser front end served from another origin needs that
  public origin (`https://app.example.com`) in `cors_origins`, both for its
  reads and for the origin guard to let its writes through.

`docker compose up` uses the same image with the `velesdb_data` volume and the
health check pre-wired.

---

## 2. Kubernetes

`velesdb-server` exposes a liveness endpoint (`/health`, always 200 while the
process lives) and a readiness endpoint (`/ready`, 503 until every collection
is loaded from disk). Wire both:

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

Because the engine takes an exclusive OS-level lock on its data directory
(`<data_dir>/velesdb.lock`), **one process owns one data directory**. Deploy as
a `StatefulSet` with a single replica per volume, not as a horizontally scaled
`Deployment` sharing one PVC. See
[CONCURRENCY_LOCKING.md](CONCURRENCY_LOCKING.md).

On `SIGTERM`, the server drains in-flight requests then flushes every
write-ahead log, so a rolling restart does not lose acknowledged writes. Give
the pod a `terminationGracePeriodSeconds` at least as large as
`[server] shutdown_timeout_secs` (default 30). Details in
[SERVER_SECURITY.md §4](SERVER_SECURITY.md#4-graceful-shutdown).

### Endpoints that bypass authentication

When API keys are configured, these four paths still answer without
credentials, so load balancers and orchestrators can probe the server:

| Endpoint | Purpose |
|----------|---------|
| `GET /health`, `GET /v1/health` | Liveness probe |
| `GET /ready`, `GET /v1/ready` | Readiness probe |

`GET /metrics` is **not** in that list: it is gated by the API key when auth is
enabled, because the Prometheus exposition leaks collection names and write
rates (see `crates/velesdb-server/src/auth.rs::is_public_path` and finding F-02
of the auth audit). Scrape it with the `Authorization: Bearer <key>` header.
The families it carries — plan cache, query counters and durations, the
engine's guard-rail rejections, per-graph-collection `velesdb_graph_*` series
labelled `collection="<name>"`, and the process-wide `velesdb_match_*` series —
are listed in the [REST reference](../reference/api-reference.md#get-metrics),
including the ones declared but not fed yet, which always read `0`.

---

## 3. Rate limiting

Per-IP rate limiting is **on by default at 100 requests/second per IP**
(`DEFAULT_RATE_LIMIT` in `crates/velesdb-server/src/config.rs`).

```bash
# Raise the ceiling
velesdb-server --rate-limit 500

# Disable entirely
velesdb-server --rate-limit 0
```

Equivalent settings: `VELESDB_RATE_LIMIT`, or `[server] rate_limit` in
`velesdb.toml`.

- Over-limit requests get `429 Too Many Requests`.
- Responses carry `x-ratelimit-limit`, `x-ratelimit-remaining`, and — on a 429 —
  `retry-after`.
- The client IP is resolved through `x-forwarded-for` / `x-real-ip` /
  `forwarded` before falling back to the peer address, so the limiter stays
  correct behind a reverse proxy. Make sure that proxy overwrites those headers
  instead of forwarding client-supplied ones.

---

## 4. CORS

CORS is **off by default**: with no origin configured the server sends no CORS
header, so only pages served from its own origin can read its responses (7.1.0
allowed every origin). A browser front end on another origin needs its origin
listed in `velesdb.toml`:

```toml
[cors]
allowed_origins = ["https://app.example.com"]
allowed_methods = ["GET", "POST"]
allowed_headers = ["Content-Type", "Authorization"]
allow_credentials = false
max_age_secs = 3600
```

Keys are read from the `[cors]` table by
`crates/velesdb-server/src/cors.rs`. `allowed_origins = ["*"]`, written
explicitly, selects the fully permissive policy (with a startup warning); an
empty list, the default, installs no CORS layer; any other list allows those
origins exactly,
each one `scheme://host[:port]` compared byte for byte with the browser's
`Origin` header. An origin with a path or a trailing `/`, `"*"` inside a list,
or a method or header name that does not parse stops the server at startup.

`[server]` carries two shorter keys for the same policy:

| Key | Default | Effect |
|-----|---------|--------|
| `cors_origins` | `[]` | The same list as `[cors] allowed_origins`; setting both is refused at startup |
| `cors_enabled` | `true` | Master switch: `false` installs no CORS layer whatever the list says |

Why the default is closed, and what allow-all exposes, is in
[SERVER_SECURITY.md](SERVER_SECURITY.md#6-cors-origins-and-hosts).

---

## 5. Startup update check

On startup, and then every 24 hours while it runs, the server sends a
non-blocking check to `https://velesdb.com/api/check` with its version, OS,
architecture, edition, a SHA-256 of a random per-install id, and whether it runs
under CI. [Update check and telemetry](CONFIGURATION.md#update-check-and-telemetry)
lists exactly what is sent, what is stored, and every way to opt out.

Disable it in air-gapped or privacy-sensitive deployments:

```bash
export VELESDB_NO_UPDATE_CHECK=1
```

or in `velesdb.toml`:

```toml
[update_check]
enabled = false
```

---

## 6. Runtime sizing, request size, logs

```toml
[server]
workers = 8                 # Tokio worker threads; 0 (default) = one per CPU
max_body_size = 268435456   # 256 MiB point-upload bodies (default 100 MiB)

[logging]
level = "info"              # VELESDB_LOGGING_LEVEL and RUST_LOG override it
format = "json"             # one JSON object per line
file = "/var/log/velesdb/velesdb.log"   # appended to; directory must exist
```

- `workers` fixes the number of threads serving requests. Unset or `0`, Tokio
  starts one per CPU the process may use (`std::thread::available_parallelism`,
  or `TOKIO_WORKER_THREADS` when that is set). The startup log prints the count
  actually running.
- `max_body_size` bounds the memory one upload request can take — the body is
  buffered whole before it is parsed. Routes other than point upload stay at
  2 MiB (lower `max_body_size` lowers them too); the NDJSON stream route is
  not buffered and has no body limit. An oversized request gets `413`.
- A key the server does not read in `[server]`, `[auth]`, `[tls]`, `[cors]` or
  `[logging]`, and a top-level table nobody reads, is named in a startup
  warning instead of being ignored silently.

Every key, its range and its precedence: [CONFIGURATION.md](CONFIGURATION.md#section-server).

---

## See also

- [velesdb-server README](../../crates/velesdb-server/README.md)
- [REST tour](SERVER_REST_TOUR.md) — the API, endpoint by endpoint, with curl
- [Server security](SERVER_SECURITY.md) — API keys, TLS, shutdown, health

---

Last updated: 2026-10-30 · Applies to: velesdb-core 8.0.0
