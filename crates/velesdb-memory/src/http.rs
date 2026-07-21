//! Streamable-HTTP transport (multi-client mode).
//!
//! `velesdb-memory` speaks stdio by default: every MCP client (Claude Code,
//! Claude Desktop, Windsurf, …) spawns its own server process, and the
//! store's single-writer `flock` (`velesdb-core`'s `Database::open_impl`)
//! then lets only ONE of those processes actually hold the store open —
//! every other client's session fails with `Storage(DatabaseLocked)`.
//!
//! This module is the fix: one process, reachable over HTTP, that several
//! clients connect to concurrently. It only builds the [`Router`]; binding a
//! [`tokio::net::TcpListener`] and driving `axum::serve` is the binary's job
//! (`src/main.rs`), so the router can also be mounted directly in tests
//! (`tests/http_transport.rs`) with no subprocess involved.
//!
//! Concurrent requests need no *application*-level locking beyond what
//! [`McpServer`] already has: `velesdb-core`'s `Database` protects its
//! collections internally with a `parking_lot::RwLock`, so many HTTP
//! sessions calling `remember`/`recall` at once are already safe. The
//! store's `flock` is untouched by this module — it still guards
//! cross-*process* access exactly as it does for stdio, which is why a
//! second `velesdb-memory --http` against the same store still fails fast
//! with the same actionable lock message (see `open_store_with_actionable_lock_error`
//! in `src/main.rs`).

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

use crate::mcp::McpServer;

/// Default bind address for `--http` / `VELESDB_MEMORY_HTTP=1` when neither
/// `VELESDB_MEMORY_HTTP_BIND` nor `--http-port` overrides it. Loopback-only:
/// this is a local multi-client daemon, not a public listener.
pub const DEFAULT_HTTP_BIND: &str = "127.0.0.1:18090";

/// Build the axum [`Router`] serving the MCP streamable-HTTP transport at
/// `/mcp` and a plain liveness probe at `/health` (used by the installer
/// script and CI to confirm the daemon is up without speaking MCP itself).
///
/// [`McpServer`] is cheaply [`Clone`] (an `Arc`-wrapped
/// [`MemoryService`](crate::service::MemoryService) internally), so the
/// `service_factory` closure below just clones the handle per session
/// rather than reopening the store.
///
/// `cancellation_token` is the caller's shutdown handle: cancelling it (or
/// any parent token it was derived from) stops accepting new HTTP-transport
/// sessions and tears down the ones in flight. The binary derives it from
/// its own shutdown token; tests derive it from a token they cancel at the
/// end of the test to stop the server cleanly.
pub fn router(server: McpServer, cancellation_token: CancellationToken) -> Router {
    let mcp_service: StreamableHttpService<McpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default().with_cancellation_token(cancellation_token),
        );
    Router::new()
        .nest_service("/mcp", mcp_service)
        .route("/health", get(health))
}

/// Liveness probe: 200 OK with no body semantics beyond "the process is up
/// and its HTTP listener is accepting requests". Deliberately doesn't touch
/// the store — a store-level health check would need a blocking read and
/// isn't what callers (the installer's `curl` wait loop, CI) are checking
/// for here.
async fn health() -> &'static str {
    "OK"
}
