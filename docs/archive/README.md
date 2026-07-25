# Archive

Documents kept for the record and no longer maintained. They describe release
lines that are several major versions behind the current one; read them only to
understand what an old upgrade involved, never as guidance for a current
install.

For anything current, start from [`docs/README.md`](../README.md).

| Document | Covers |
|---|---|
| [`MIGRATION_v1.6.md`](MIGRATION_v1.6.md) | Upgrading v1.5 → v1.6 — opt-in server security (API keys, TLS, graceful shutdown) |
| [`MIGRATION_v1.7.md`](MIGRATION_v1.7.md) | Upgrading v1.6 → v1.7 — HNSW upsert semantics, opt-in GPU acceleration, chunked batch insert |
| [`CLI_REPL.md`](CLI_REPL.md) | The former combined CLI & REPL guide — superseded, and inaccurate on the current command surface |

Redirect stubs remain at the original paths under `docs/guides/`, so links
published before the move keep working.

Recent migrations stay in `docs/guides/` — the latest is
[`MIGRATION_v3.3.0.md`](../guides/MIGRATION_v3.3.0.md).

---
Last updated: 2026-07-25 · Applies to: velesdb-core 4.0.0
