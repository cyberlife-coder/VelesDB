//! Transport selection and daemon lifecycle: stdio vs HTTP serving, the
//! loopback gate, logging, shutdown signals, and the orphan watchdog.

use rmcp::ServiceExt;
use velesdb_memory::mcp::McpServer;

pub(crate) async fn serve_stdio(
    server: McpServer,
    original_parent: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    spawn_orphan_watchdog(original_parent);
    let running = server
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    running.waiting().await?;
    Ok(())
}

#[cfg(feature = "http")]
pub(crate) async fn serve_selected_transport(
    server: McpServer,
    http_bind: Option<HttpServeRequest>,
    original_parent: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(request) = http_bind {
        return serve_http(server, request).await;
    }
    serve_stdio(server, original_parent).await
}

#[cfg(not(feature = "http"))]
pub(crate) async fn serve_selected_transport(
    server: McpServer,
    _http_bind: Option<String>,
    original_parent: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    serve_stdio(server, original_parent).await
}

/// Install the `VELESDB_MEMORY_LOG` subscriber, or exit with its actionable
/// message. The exit path (prefixed `eprintln!` + status 1) matches every
/// other startup refusal in this binary (`requested_http_bind`,
/// `open_store_with_actionable_lock_error`) — bubbling the `String` up
/// through `main`'s `Result` would print it as an unprefixed `Debug` quote
/// instead.
pub(crate) fn apply_logging() {
    if let Err(err) = velesdb_memory::logging::init_from_env() {
        eprintln!("[velesdb-memory] {err}");
        std::process::exit(1);
    }
}

/// A resolved `--http`/`VELESDB_MEMORY_HTTP=1` request: where to bind, and
/// whether TLS should be skipped (`--http-insecure` /
/// `VELESDB_MEMORY_HTTP_INSECURE=1` — see [`requested_http_bind`]).
#[cfg(feature = "http")]
pub(crate) struct HttpServeRequest {
    bind_addr: String,
    insecure: bool,
}

/// Detect the streamable-HTTP transport request (`--http` flag or
/// `VELESDB_MEMORY_HTTP=1`) and resolve how it should be served, BEFORE the
/// store is opened. Returns `None` for the default stdio transport.
///
/// Without the `http` feature, `--http`/`VELESDB_MEMORY_HTTP=1` is rejected
/// with an actionable message instead of silently falling back to stdio —
/// the binary was built without the code to honor the request at all.
#[cfg(feature = "http")]
pub(crate) fn requested_http_bind(args: &[String]) -> Option<HttpServeRequest> {
    let http_flag = args.iter().any(|arg| arg == "--http");
    let http_env = std::env::var("VELESDB_MEMORY_HTTP").as_deref() == Ok("1");
    if !http_flag && !http_env {
        return None;
    }

    let port_override = args
        .iter()
        .position(|arg| arg == "--http-port")
        .and_then(|flag_index| args.get(flag_index + 1));

    let default_bind = std::env::var("VELESDB_MEMORY_HTTP_BIND")
        .unwrap_or_else(|_| velesdb_memory::http::DEFAULT_HTTP_BIND.to_owned());

    let bind_addr = match port_override {
        Some(port) => match default_bind.rsplit_once(':') {
            Some((host, _existing_port)) => format!("{host}:{port}"),
            None => format!("127.0.0.1:{port}"),
        },
        None => default_bind,
    };

    // The router (`velesdb_memory::http::router`) authenticates no one: any
    // caller that can reach the socket gets full `remember`/`recall`/`relate`
    // access to the store. That's only safe because the default bind is
    // loopback-only. `VELESDB_MEMORY_HTTP_BIND` lets the *port* be
    // overridden freely, but overriding the *host* to something reachable
    // off-box would turn an unauthenticated local daemon into an
    // unauthenticated network service — so that requires an explicit,
    // separate opt-in rather than falling out of a bind-address typo.
    if !is_loopback_host(&bind_addr)
        && std::env::var("VELESDB_MEMORY_HTTP_ALLOW_REMOTE").as_deref() != Ok("1")
    {
        eprintln!(
            "[velesdb-memory] refusing to bind the HTTP transport to '{bind_addr}': it is not a \
             loopback address, and the streamable-HTTP transport has no authentication — anyone \
             who can reach that socket gets full read/write access to the store. Set \
             VELESDB_MEMORY_HTTP_ALLOW_REMOTE=1 to override (put an authenticating reverse proxy \
             in front first)."
        );
        std::process::exit(1);
    }

    // `--http-insecure` / `VELESDB_MEMORY_HTTP_INSECURE=1` is the explicit
    // opt-out of HTTPS-by-default (see the crate-level doc comment above and
    // `velesdb_memory::tls`'s module docs for why HTTPS is the default at
    // all) — an "insecure escape hatch, loud at startup" flag, same shape as
    // `VELESDB_MEMORY_HTTP_ALLOW_REMOTE` above. Kept as its own flag rather
    // than folded into that one: that one is about *who* can reach the
    // socket, this one is about *whether the bytes on the wire are
    // encrypted* — independent axes, and conflating them would make an
    // operator who only wants one silently get the other too.
    let insecure_flag = args.iter().any(|arg| arg == "--http-insecure");
    let insecure_env = std::env::var("VELESDB_MEMORY_HTTP_INSECURE").as_deref() == Ok("1");
    let insecure = insecure_flag || insecure_env;

    Some(HttpServeRequest {
        bind_addr,
        insecure,
    })
}

/// Whether `bind_addr`'s host component (`host:port` or `[ipv6]:port`)
/// resolves to a loopback address. Used to gate non-local HTTP binds behind
/// an explicit opt-in — see `requested_http_bind` above. An unparseable host
/// (e.g. a hostname like `mcp.example.com` rather than a literal IP) is
/// treated as non-loopback: `TcpListener::bind` does its own DNS resolution
/// later, so this is a conservative pre-check, not the only one.
#[cfg(feature = "http")]
pub(crate) fn is_loopback_host(bind_addr: &str) -> bool {
    let host = bind_addr
        .rsplit_once(':')
        .map_or(bind_addr, |(host, _port)| host)
        .trim_start_matches('[')
        .trim_end_matches(']');
    host.parse::<std::net::IpAddr>()
        .is_ok_and(|ip| ip.is_loopback())
}

/// See the `http`-feature variant above. Without `http`, no bind address can
/// ever be resolved — the binary has no HTTP transport built in — so a
/// `--http`/`VELESDB_MEMORY_HTTP=1` request fails fast with guidance instead
/// of being silently ignored (which would otherwise look like the server
/// just hung, or served the wrong transport).
#[cfg(not(feature = "http"))]
pub(crate) fn requested_http_bind(args: &[String]) -> Option<String> {
    let http_flag = args.iter().any(|arg| arg == "--http");
    let http_env = std::env::var("VELESDB_MEMORY_HTTP").as_deref() == Ok("1");
    if http_flag || http_env {
        eprintln!(
            "[velesdb-memory] --http / VELESDB_MEMORY_HTTP=1 requires a binary built with \
             `--features http` (e.g. `cargo install velesdb-memory --features http`) — \
             this binary was built without it"
        );
        std::process::exit(1);
    }
    None
}

/// Serve the MCP server over the streamable-HTTP transport (multi-client
/// mode): binds `request.bind_addr`, mounts [`velesdb_memory::http::router`],
/// and runs until either the process receives Ctrl-C or the returned future
/// is dropped (e.g. process termination) — a background daemon (launchd,
/// systemd) is expected to just kill the process on stop, which is safe: the
/// store's `flock` is released by the kernel on exit regardless (see the
/// orphan-watchdog docs above).
///
/// HTTPS by default (a locally-generated CA + leaf cert — see
/// `velesdb_memory::tls`), unless `request.insecure` opts out
/// (`--http-insecure` / `VELESDB_MEMORY_HTTP_INSECURE=1`), in which case
/// this serves plain HTTP via `axum::serve` exactly as before HTTPS
/// support was added.
#[cfg(feature = "http")]
pub(crate) async fn serve_http(
    server: McpServer,
    request: HttpServeRequest,
) -> Result<(), Box<dyn std::error::Error>> {
    let HttpServeRequest {
        bind_addr,
        insecure,
    } = request;

    let ct = tokio_util::sync::CancellationToken::new();
    let app = velesdb_memory::http::router(server, ct.child_token());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    spawn_shutdown_signals(ct.clone());

    if insecure {
        eprintln!(
            "[velesdb-memory] WARNING: --http-insecure / VELESDB_MEMORY_HTTP_INSECURE=1 is set — \
             serving PLAIN HTTP (no TLS) on http://{bind_addr}/mcp. Every request is readable by \
             anyone who can reach that socket (loopback-only by default — see \
             VELESDB_MEMORY_HTTP_ALLOW_REMOTE above). Use this only for local debugging, or when \
             a trusted TLS-terminating proxy already sits in front."
        );
        eprintln!("[velesdb-memory] HTTP server listening on http://{bind_addr}/mcp");
        axum::serve(listener, app)
            .with_graceful_shutdown(async move { ct.cancelled_owned().await })
            .await?;
        return Ok(());
    }

    let tls_dir = velesdb_memory::tls::tls_dir_from_env();
    let material = velesdb_memory::tls::ensure_tls_material(&tls_dir)?;
    let acceptor = velesdb_memory::tls::tls_acceptor_from_material(&material)?;
    eprintln!("[velesdb-memory] HTTPS server listening on https://{bind_addr}/mcp");
    eprintln!(
        "[velesdb-memory] Local CA: {} — a client only needs to trust this once (see \
         ./scripts/install-memory-daemon.sh, which does this automatically on macOS); every \
         future leaf certificate this daemon issues is signed by the same CA and is trusted \
         automatically after that.",
        material.ca_cert_path.display()
    );

    velesdb_memory::http::serve_tls(app, listener, acceptor, ct).await;
    Ok(())
}

/// How often the orphan watchdog re-checks its parent pid. The MCP stdio
/// transport only observes disconnects via stdin EOF, which a client that
/// leaks its child process (the #1448 scenario) never delivers — so this is
/// the *only* signal that would otherwise catch that leak. 2s keeps the
/// worst-case self-exit latency low (a handful of polls) without burning
/// meaningful CPU on an idle server.
#[cfg(unix)]
pub(crate) const ORPHAN_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// Detect a dead parent and self-exit, releasing the store's `flock` even
/// when stdin is artificially kept open (a leaked child process, #1448).
///
/// A normal MCP stdio client closes the child's stdin on disconnect, which
/// the existing EOF path already handles. But a client that merely forgets
/// to reap/close its child (observed in practice: a headless `claude -p`
/// run left its server running) never closes that pipe — the server then
/// legitimately keeps serving forever, holding the single-writer store lock
/// and making every later session fail `Storage(DatabaseLocked)`.
///
/// This has no other shutdown trigger to lean on, so it polls: capture the
/// parent pid at startup, and if it ever changes, the parent is gone (Unix
/// re-parents orphans to init/launchd, pid 1 or the user's launchd pid —
/// never the original parent), so exit. `std::os::unix::process::parent_id`
/// is pure `std`, avoiding a new dependency (e.g. `libc::getppid`) for a
/// single syscall.
///
/// Process exit (even via `std::process::exit`, which skips destructors)
/// still releases the store's `flock`: that lock is a kernel-held resource
/// tied to the process's open file descriptors, which the kernel closes —
/// and therefore unlocks — unconditionally on process exit, confirmed by
/// the investigation on #1448 ("released by the kernel even on SIGKILL").
#[cfg(unix)]
pub(crate) fn spawn_orphan_watchdog(original_parent: u32) {
    use std::os::unix::process::parent_id;

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(ORPHAN_CHECK_INTERVAL).await;
            let current_parent = parent_id();
            if current_parent != original_parent {
                eprintln!(
                    "[velesdb-memory] parent process (pid {original_parent}) is gone \
                     (now reparented under pid {current_parent}) — exiting to release \
                     the store lock rather than leak a zombie session (#1448)"
                );
                std::process::exit(0);
            }
        }
    });
}

/// Windows has no equivalent of `parent_id()` re-parenting to detect a dead
/// parent this cheaply, so this hardening is Unix-only for now — behavior on
/// Windows is unchanged (still relies on the stdin-EOF path).
#[cfg(not(unix))]
pub(crate) fn spawn_orphan_watchdog(_original_parent: u32) {}

/// Cancel `ct` on the signals a supervisor actually sends.
///
/// SIGINT alone is not enough. `launchctl kickstart -k`, `systemctl restart`
/// and `docker stop` all send **SIGTERM**, and an unhandled SIGTERM kills the
/// process outright: the streamable-HTTP sessions clients hold are dropped
/// mid-flight, so the next call on a live session hangs until the client's own
/// timeout instead of reconnecting. Handling it lets `with_graceful_shutdown`
/// close those sessions, which is what turns a restart into a reconnect.
#[cfg(feature = "http")]
pub(crate) fn spawn_shutdown_signals(ct: tokio_util::sync::CancellationToken) {
    let interrupt = ct.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            interrupt.cancel();
        }
    });

    #[cfg(unix)]
    tokio::spawn(async move {
        // `signal()` only fails if the handler cannot be registered at all; a
        // daemon that cannot listen for SIGTERM still serves, it just loses the
        // graceful path, so this is not worth aborting startup for.
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            term.recv().await;
            ct.cancel();
        }
    });
    // On Windows there is no SIGTERM; Ctrl-C above is the whole contract.
    #[cfg(not(unix))]
    drop(ct);
}

#[cfg(all(test, feature = "http"))]
#[path = "daemon_serve_tests.rs"]
mod tests;
