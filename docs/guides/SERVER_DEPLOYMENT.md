# VelesDB Server — Deployment

How to run `velesdb-server` outside a developer laptop: containers,
orchestrators, probes, rate limiting, and CORS.

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
| `RUST_LOG` | `info` |

It runs as the non-root `velesdb` user, exposes port 8080, and declares a
`HEALTHCHECK` that curls `/health` every 30 s.

> **`VELESDB_HOST=0.0.0.0` binds a publicly reachable address.** With no API
> key configured the server logs a warning at startup and every endpoint —
> including `/metrics` and all data — is reachable by anyone who can reach the
> container. Set `VELESDB_API_KEYS` before exposing it; see
> [SERVER_SECURITY.md](SERVER_SECURITY.md).

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

CORS defaults to **permissive** (`allowed_origins = ["*"]`), which is why the
server logs a warning at startup. Restrict it in `velesdb.toml`:

```toml
[cors]
allowed_origins = ["https://app.example.com"]
allowed_methods = ["GET", "POST"]
allowed_headers = ["Content-Type", "Authorization"]
allow_credentials = false
max_age_secs = 3600
```

Keys are read from the `[cors]` table by
`crates/velesdb-server/src/config.rs`. `allowed_origins = ["*"]` selects the
fully permissive policy; any other list restricts to those origins exactly.

---

## 5. Startup update check

On startup the server performs a non-blocking version check against
`https://velesdb.com/api/check`. It sends only the version, OS, architecture,
and a non-reversible SHA-256 instance hash — no personal data.

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

## See also

- [velesdb-server README](../../crates/velesdb-server/README.md)
- [REST tour](SERVER_REST_TOUR.md) — the API, endpoint by endpoint, with curl
- [Server security](SERVER_SECURITY.md) — API keys, TLS, shutdown, health

---

Last updated: 2026-07-25 · Applies to: velesdb-core 5.0.0
