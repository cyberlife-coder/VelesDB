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
use tower_http::limit::RequestBodyLimit;

use crate::mcp::McpServer;

mod session_limit;

use session_limit::BoundedSessionManager;

/// Default bind address for `--http` / `VELESDB_MEMORY_HTTP=1` when neither
/// `VELESDB_MEMORY_HTTP_BIND` nor `--http-port` overrides it. Loopback-only:
/// this is a local multi-client daemon, not a public listener.
pub const DEFAULT_HTTP_BIND: &str = "127.0.0.1:18090";

/// Default max size (bytes) of a single `/mcp` HTTP request body when
/// `VELESDB_MEMORY_HTTP_MAX_BODY_BYTES` is unset — 16 MiB. Generous headroom
/// above the largest single field cap enforced deeper in the stack
/// ([`crate::limits::MAX_TRANSCRIPT_BYTES`], 8 MiB) to cover JSON-RPC framing
/// and multi-field payloads, while still bounding the raw allocation an
/// unauthenticated-by-design loopback client can force before any
/// application-level check ever runs (see [`RequestBodyLimit`] in [`router`]).
pub const DEFAULT_HTTP_MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Resolve the `/mcp` request body limit from
/// `VELESDB_MEMORY_HTTP_MAX_BODY_BYTES`. Unset, unparseable, or `0` falls
/// back to [`DEFAULT_HTTP_MAX_BODY_BYTES`] — a `0` limit would reject every
/// request, including `initialize`, bricking the daemon.
#[must_use]
pub fn http_max_body_bytes_from_env() -> usize {
    std::env::var("VELESDB_MEMORY_HTTP_MAX_BODY_BYTES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&bytes| bytes > 0)
        .unwrap_or(DEFAULT_HTTP_MAX_BODY_BYTES)
}

/// Default max number of concurrent MCP sessions when
/// `VELESDB_MEMORY_HTTP_MAX_SESSIONS` is unset — 64. This is a local
/// multi-client daemon (a handful of editors/agents on one machine), not a
/// public service, so this is generous headroom rather than a tight budget;
/// its purpose is only to put a ceiling on `LocalSessionManager`'s session
/// map, which [`rmcp`] otherwise grows without bound (see
/// [`session_limit`] for the full rationale).
pub const DEFAULT_HTTP_MAX_SESSIONS: usize = 64;

/// Resolve the max concurrent session count from
/// `VELESDB_MEMORY_HTTP_MAX_SESSIONS`. Unset, unparseable, or `0` falls back
/// to [`DEFAULT_HTTP_MAX_SESSIONS`] — a `0` limit would reject every session,
/// including the first, bricking the daemon.
#[must_use]
pub fn http_max_sessions_from_env() -> usize {
    std::env::var("VELESDB_MEMORY_HTTP_MAX_SESSIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_HTTP_MAX_SESSIONS)
}

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
///
/// Two DoS guards wrap the `/mcp` service, both absent from rmcp's own
/// defaults (see each item's doc comment for why they matter and why the
/// obvious axum-level fix does not apply to a raw `nest_service`):
/// - [`RequestBodyLimit`] bounds a single request body
///   ([`http_max_body_bytes_from_env`]).
/// - [`BoundedSessionManager`] bounds concurrent sessions
///   ([`http_max_sessions_from_env`]).
pub fn router(server: McpServer, cancellation_token: CancellationToken) -> Router {
    router_with_limits(
        server,
        cancellation_token,
        http_max_body_bytes_from_env(),
        http_max_sessions_from_env(),
    )
}

/// [`router`], but with the two DoS guards' limits passed explicitly instead
/// of read from the environment. `router` itself is the thin, env-reading
/// wrapper adversarial tests (`tests/http_transport.rs`) skip in favor of
/// this — process-wide env vars are shared, mutable global state, and
/// `cargo test` runs a crate's tests in parallel by default, so a test that
/// wants a tiny `max_body_bytes`/`max_sessions` to actually exercise a
/// rejection would otherwise race every other test reading the same
/// variables in the same process.
#[doc(hidden)]
pub fn router_with_limits(
    server: McpServer,
    cancellation_token: CancellationToken,
    max_body_bytes: usize,
    max_sessions: usize,
) -> Router {
    let session_manager = BoundedSessionManager::new(LocalSessionManager::default(), max_sessions);
    let mcp_service: StreamableHttpService<McpServer, BoundedSessionManager<LocalSessionManager>> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Arc::new(session_manager),
            StreamableHttpServerConfig::default().with_cancellation_token(cancellation_token),
        );
    Router::new()
        .nest_service("/mcp", RequestBodyLimit::new(mcp_service, max_body_bytes))
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
