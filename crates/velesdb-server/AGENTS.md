# AGENTS.md — velesdb-server

Axum HTTP API over `velesdb-core`. Root [`AGENTS.md`](../../AGENTS.md) is the
contract; this file holds what only applies here.

- **The security surface is opt-in but never weakened.** Bearer auth
  (`src/auth.rs`), TLS via rustls, CORS (`src/cors.rs`) and the Host guard
  against DNS rebinding (`src/host_guard.rs`) are off by default for local
  dev — when a config turns them on they are the boundary: no change may
  bypass or loosen them. The contract is
  [docs/guides/SERVER_SECURITY.md](../../docs/guides/SERVER_SECURITY.md).
- **OpenAPI snapshots are committed and drift-checked.** Handler or route
  changes must regenerate `docs/openapi.json` and `docs/openapi.yaml`
  (`tests::generate_openapi_spec_files -- --exact`, features `openapi` +
  `prometheus`) — the `openapi-drift` CI job fails on drift.
- **Conformance is a gate.** The `velesql-conformance` CI job runs the
  VelesQL cargo suites plus an npm contract fixture; handler changes keep it
  green.
- **Handlers fire no observer telemetry.** Core reports every write to a
  registered collection (`on_upsert`) and every completed search or VelesQL
  read (`on_query`) itself; a handler that also calls the deprecated
  `notify_upsert`/`notify_query` double-counts. A search with no `GatedRead`
  leaf runs through `Database::run_gated_read`, never `authorize_read` plus
  its own timing. A missing event is a core defect, fixed in core.
- **Tests:** unit tests beside their module as `*_tests.rs`, integration
  suites in `tests/` — run single-threaded (`--test-threads=1`).
