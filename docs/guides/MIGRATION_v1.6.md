# Migration Guide: v1.5 → v1.6

> **Archive notice:** This guide is for upgrading from v1.5 to v1.6. Current version is v1.11.1. For the latest API, see [VELESQL_SPEC.md](../VELESQL_SPEC.md).

**Time to upgrade: under 5 minutes.**

## Zero Breaking Changes

v1.6 introduces **opt-in server security features**. All new functionality is disabled by default. Your existing setup works exactly as before — no code changes, no config changes, no data migration.

| Feature | Default | Breaking? |
|---------|---------|-----------|
| API Key Authentication | Disabled | No |
| TLS (HTTPS) | Disabled (plain HTTP) | No |
| Graceful Shutdown | Enabled automatically | No (improves safety) |
| `GET /ready` endpoint | Available | No (new endpoint) |
| `GET /health` response change | `"ok"` (was `"healthy"`) | Minor* |

\* If you parse the `/health` response body, update `"healthy"` → `"ok"`. The HTTP status code (200) is unchanged.

---

## What's New

### 1. Graceful Shutdown (automatic)

The server now handles SIGTERM and SIGINT signals gracefully:

- Stops accepting new connections
- Drains active requests (30s timeout)
- Flushes all WALs to disk
- Logs "Shutdown complete" before exit

**No action needed** — this is always active. Your data is now safer during restarts.

### 2. API Key Authentication (opt-in)

To enable authentication on an existing server:

```bash
# Set one or more API keys
export VELESDB_API_KEYS="your-secret-key"

# Restart the server
velesdb-server
```

Or add to your `velesdb.toml`:

```toml
[auth]
api_keys = ["your-secret-key"]
```

All endpoints except `/health` and `/ready` will now require:

```
Authorization: Bearer your-secret-key
```

### 3. TLS (opt-in)

To add HTTPS to an existing HTTP server:

```bash
# Generate a certificate (dev)
mkcert -cert-file cert.pem -key-file key.pem localhost

# Start with TLS
export VELESDB_TLS_CERT=cert.pem
export VELESDB_TLS_KEY=key.pem
velesdb-server
```

Or add to your `velesdb.toml`:

```toml
[tls]
cert = "cert.pem"
key = "key.pem"
```

### 4. Readiness Endpoint (new)

`GET /ready` returns 200 when the database is fully loaded, 503 during startup. Useful for monitoring scripts:

```bash
curl http://localhost:8080/ready
# {"status": "ready", "version": "1.6.0"}
```

---

## Step-by-Step Upgrade

1. **Replace the binary** (or `cargo install velesdb-server`).
2. **Start the server** — it works immediately with your existing data and config.
3. **(Optional)** Enable auth by setting `VELESDB_API_KEYS`.
4. **(Optional)** Enable TLS by setting `VELESDB_TLS_CERT` and `VELESDB_TLS_KEY`.

That's it.

---

## FAQ

### Will my existing data still work?

Yes. The storage format is unchanged. No data migration is needed.

### Do I need to update my velesdb.toml?

No. The existing config continues to work. New `[auth]` and `[tls]` sections are optional.

### Will my clients break if I enable auth?

Clients without a valid `Authorization: Bearer <key>` header will receive `401 Unauthorized`. Enable auth only after updating all clients to send the header.

### Does graceful shutdown change my restart behavior?

The server now takes up to 30 seconds to shut down (to drain active connections and flush WALs). If you need faster shutdown, the drain timeout is configurable via `shutdown_timeout_secs` in `velesdb.toml`.

### My monitoring checks GET /health — will it break?

The HTTP status code is still 200. The JSON body changed from `{"status": "healthy"}` to `{"status": "ok", "version": "..."}`. Update your parser if you check the body string.

### Where is the full documentation?

- [Server Security Guide](SERVER_SECURITY.md) — auth, TLS, shutdown, health endpoints
- [Configuration Reference](CONFIGURATION.md) — all options with env vars, CLI flags, TOML keys
