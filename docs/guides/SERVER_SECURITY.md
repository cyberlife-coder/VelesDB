# Server Security & Operations Guide

VelesDB is a **local-first** embedded database. The `velesdb-server` binary exposes VelesDB over HTTP on your local network or as a backend for your application. All security features are **opt-in** — by default the server runs without authentication or TLS, which is ideal for local development.

This guide covers the server's operational features: authentication, TLS, graceful shutdown, and health monitoring.

> **See also:** [CONFIGURATION.md](CONFIGURATION.md) for the full configuration reference (all options, env vars, CLI flags, TOML keys).

---

## Table of Contents

1. [Running the Server](#1-running-the-server)
2. [Authentication Setup](#2-authentication-setup)
3. [TLS Setup](#3-tls-setup)
4. [Graceful Shutdown](#4-graceful-shutdown)
5. [Health Endpoints](#5-health-endpoints)

---

## 1. Running the Server

### From a release binary

```bash
./velesdb-server
```

The server starts on `127.0.0.1:8080` with data stored in `./velesdb_data/`. No authentication, no TLS — ready for local development.

### From source

```bash
cargo run -p velesdb-server
```

### With a configuration file

```bash
velesdb-server --config velesdb.toml
```

Or via environment variable:

```bash
export VELESDB_CONFIG=./velesdb.toml
velesdb-server
```

### Common startup options

```bash
# Custom port and data directory
velesdb-server --port 9090 --data-dir /var/lib/velesdb

# Equivalent with env vars
VELESDB_PORT=9090 VELESDB_DATA_DIR=/var/lib/velesdb velesdb-server
```

### Configuration priority

The server merges configuration from multiple sources. **Highest priority wins:**

```
CLI flags / Environment variables  >  TOML file  >  Defaults
```

| Option | CLI flag | Env var | Default |
|--------|----------|---------|---------|
| Bind address | `--host` | `VELESDB_HOST` | `127.0.0.1` |
| Port | `--port` | `VELESDB_PORT` | `8080` |
| Data directory | `--data-dir` | `VELESDB_DATA_DIR` | `./velesdb_data` |
| Config file | `--config` | `VELESDB_CONFIG` | *(none)* |

See [CONFIGURATION.md](CONFIGURATION.md) for the complete option list.

---

## 2. Authentication Setup

VelesDB supports **Bearer token authentication** via API keys. When enabled, all endpoints except health checks require a valid `Authorization: Bearer <key>` header.

### Enabling authentication

**Option A: Environment variable (recommended for quick setup)**

```bash
# Single key
export VELESDB_API_KEYS="my-secret-key-1"

# Multiple keys (comma-separated)
export VELESDB_API_KEYS="key-for-app-a,key-for-app-b,key-for-admin"

velesdb-server
```

**Option B: Configuration file**

```toml
# velesdb.toml
[auth]
api_keys = [
    "key-for-app-a",
    "key-for-app-b",
    "key-for-admin",
]
```

### Making authenticated requests

```bash
curl -H "Authorization: Bearer key-for-app-a" \
     http://localhost:8080/api/v1/collections
```

### Unauthenticated responses

When auth is enabled, requests without a valid token receive:

```http
HTTP/1.1 401 Unauthorized
Content-Type: application/json

{"error": "Unauthorized", "message": "invalid API key"}
```

### Public endpoints (always accessible)

These endpoints **bypass authentication**, even when API keys are configured:

- `GET /health` — Liveness probe
- `GET /ready` — Readiness probe

### Key rotation best practices

VelesDB supports multiple simultaneous API keys, which enables zero-downtime key rotation:

1. **Add the new key** alongside the existing one:
   ```bash
   export VELESDB_API_KEYS="old-key,new-key"
   ```
2. **Restart the server** — both keys are now valid.
3. **Update your clients** to use the new key.
4. **Remove the old key** once all clients have migrated:
   ```bash
   export VELESDB_API_KEYS="new-key"
   ```
5. **Restart the server** to revoke the old key.

> **Tip:** Use long, random strings for API keys (e.g., `openssl rand -hex 32`). Treat them like passwords — never commit them to version control.

### Disabling authentication

Authentication is **disabled by default**. If you previously enabled it, simply remove the `VELESDB_API_KEYS` env var or the `[auth]` section from your TOML file and restart the server.

---

## 3. TLS Setup

VelesDB supports HTTPS via **rustls** (a pure-Rust TLS implementation). When TLS is configured, the server binds with HTTPS. Otherwise, it serves plain HTTP.

### Enabling TLS

You need a PEM-encoded certificate file and a PEM-encoded private key file.

**Option A: Environment variables**

```bash
export VELESDB_TLS_CERT=/path/to/cert.pem
export VELESDB_TLS_KEY=/path/to/key.pem
velesdb-server
```

**Option B: CLI flags**

```bash
velesdb-server --tls-cert /path/to/cert.pem --tls-key /path/to/key.pem
```

**Option C: Configuration file**

```toml
# velesdb.toml
[tls]
cert = "/path/to/cert.pem"
key = "/path/to/key.pem"
```

### Generating certificates for development

Use [mkcert](https://github.com/FiloSottile/mkcert) to create locally-trusted certificates:

```bash
# Install the local CA (one-time)
mkcert -install

# Generate cert for localhost
mkcert -cert-file cert.pem -key-file key.pem localhost 127.0.0.1 ::1

# Start with TLS
velesdb-server --tls-cert cert.pem --tls-key key.pem
```

Your browser and tools will trust these certificates automatically on your machine.

### Generating certificates for production (local network)

For exposing VelesDB on a local network, use [Let's Encrypt](https://letsencrypt.org/) with a domain name, or generate a self-signed certificate:

```bash
# Self-signed certificate (valid 365 days)
openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem \
  -days 365 -nodes -subj "/CN=velesdb.local"
```

> **Note:** Clients connecting with self-signed certificates will need to trust the CA or disable certificate verification.

### TLS validation

The server validates TLS configuration at startup and will refuse to start if:

- Only one of `cert` or `key` is provided (both are required).
- The certificate or key file does not exist.
- The files contain invalid PEM data.

Error messages are clear and specific — check the server logs if startup fails.

### Complete example: Auth + TLS

```toml
# velesdb.toml
[server]
host = "0.0.0.0"
port = 8443

[auth]
api_keys = ["my-secure-key"]

[tls]
cert = "./certs/cert.pem"
key = "./certs/key.pem"
```

```bash
curl -H "Authorization: Bearer my-secure-key" \
     https://localhost:8443/api/v1/collections
```

---

## 4. Graceful Shutdown

VelesDB handles shutdown signals gracefully to prevent data loss.

### Shutdown sequence

When the server receives **SIGTERM** or **SIGINT** (Ctrl+C):

1. **Stop accepting new connections** — the listening socket closes.
2. **Drain active requests** — in-flight requests are allowed to complete, with a **30-second timeout**. Requests that exceed the timeout are dropped.
3. **Flush all WALs** — the server calls `Database::flush_all()` to persist all write-ahead logs to disk. This covers all collection types (vector, graph, metadata).
4. **Log shutdown status** — the server logs "Shutdown complete" before exiting.
5. **Exit cleanly** — the process exits with code 0.

### WAL flush guarantee

The graceful shutdown ensures that **all data written before the shutdown signal is persisted to disk**. This is the key safety guarantee: you will not lose acknowledged writes due to a clean shutdown.

If a WAL flush fails for a specific collection, the server logs a warning but continues shutting down other collections. Check the logs for any flush warnings.

### Shutdown timeout

The connection drain timeout is **30 seconds**. Connections that are still active after this period are terminated. This prevents a single slow client from blocking shutdown indefinitely.

### Signals

| Signal | Trigger | Behavior |
|--------|---------|----------|
| `SIGINT` | Ctrl+C in terminal | Graceful shutdown |
| `SIGTERM` | Process manager / `kill` | Graceful shutdown |
| `SIGKILL` | `kill -9` | **Immediate termination** — no WAL flush, possible data loss |

> **Avoid `kill -9`** unless the server is truly unresponsive. Always prefer `SIGTERM` to allow WAL flushing.

---

## 5. Health Endpoints

VelesDB provides two health endpoints for monitoring and orchestration. Both bypass authentication.

### GET /health (Liveness)

Returns 200 OK as long as the server process is running. Use this to check if the server is alive.

```bash
curl http://localhost:8080/health
```

```json
{
  "status": "ok",
  "version": "1.11.1"
}
```

This endpoint **always returns 200** — if it doesn't respond, the process is down.

### GET /ready (Readiness)

Returns 200 OK only when the database is fully loaded and ready to serve requests. Returns 503 during startup (while collections are loading from disk).

```bash
curl http://localhost:8080/ready
```

**Ready (200):**

```json
{
  "status": "ready",
  "version": "1.11.1"
}
```

**Not ready (503):**

```json
{
  "status": "not_ready",
  "version": "1.11.1"
}
```

### Monitoring usage

Use these endpoints with your monitoring tool of choice:

```bash
# Simple health check script
while true; do
  STATUS=$(curl -s -o /dev/null -w "%{http_code}" http://localhost:8080/ready)
  if [ "$STATUS" -ne 200 ]; then
    echo "VelesDB is not ready (HTTP $STATUS)"
  fi
  sleep 10
done
```

### Startup check

Wait for the server to be ready before sending traffic:

```bash
# Wait for readiness after starting the server
velesdb-server &
until curl -sf http://localhost:8080/ready > /dev/null 2>&1; do
  echo "Waiting for VelesDB..."
  sleep 1
done
echo "VelesDB is ready!"
```
