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
- **Handlers fire no write telemetry.** Core reports every write to a
  registered collection to the `DatabaseObserver` itself (`on_upsert`); a
  handler that also calls the deprecated `notify_upsert` double-counts. A
  missing event is a core defect, fixed in core.
- **Tests:** unit tests beside their module as `*_tests.rs`, integration
  suites in `tests/` — run single-threaded (`--test-threads=1`).
