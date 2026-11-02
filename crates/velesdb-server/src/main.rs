#![warn(clippy::significant_drop_tightening)]
#![allow(clippy::doc_markdown)]
//! `VelesDB` Server - REST API for the `VelesDB` vector database.

use axum::serve::ListenerExt;
use axum::{extract::ConnectInfo, http::HeaderValue, middleware::Next, Router};
use clap::Parser;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;
use tower_http::trace::TraceLayer;

#[cfg(feature = "swagger-ui")]
use utoipa_swagger_ui::SwaggerUi;
use velesdb_core::metrics::OperationalMetrics;
use velesdb_core::Database;
use velesdb_server::connections::{CountedStream, CountingListener, OpenConnection};
#[cfg(feature = "swagger-ui")]
use velesdb_server::ApiDoc;
use velesdb_server::{
    auth::{auth_middleware, AuthState},
    config::{
        apply_browser_policy, load_core_config, parse_api_keys_env, CliOverrides, CorsConfig,
        ServerConfig,
    },
    routes::api_routes_with_body_limit,
    AppState, OnboardingMetrics,
};

/// VelesDB Server - A high-performance vector database
#[derive(Parser, Debug)]
#[command(name = "velesdb-server")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to velesdb.toml configuration file. Configures both the
    /// server ([server]/[auth]/[tls]/[cors]/[logging]) and the core engine
    /// ([search]/[hnsw]/[storage]/[limits]/[quantization]) —
    /// only these engine sections are read into the core config, so a
    /// same-named [server]/[logging] table stays exclusively the
    /// server's own. VELESDB_* env vars still override values from the
    /// file. An invalid or missing explicit path fails fast at startup —
    /// never a silent fallback to defaults.
    #[arg(short, long, env = "VELESDB_CONFIG")]
    config: Option<PathBuf>,

    /// Data directory for persistent storage
    #[arg(short, long, env = "VELESDB_DATA_DIR")]
    data_dir: Option<String>,

    /// Host address to bind to
    #[arg(long, env = "VELESDB_HOST")]
    host: Option<String>,

    /// Port to listen on
    #[arg(short, long, env = "VELESDB_PORT")]
    port: Option<u16>,

    /// TLS certificate file (PEM)
    #[arg(long, env = "VELESDB_TLS_CERT")]
    tls_cert: Option<String>,

    /// TLS private key file (PEM)
    #[arg(long, env = "VELESDB_TLS_KEY")]
    tls_key: Option<String>,

    /// Rate limit: max requests per second per IP (0 = disabled)
    #[arg(long, env = "VELESDB_RATE_LIMIT")]
    rate_limit: Option<u32>,

    /// Host names to answer to besides localhost, 127.0.0.0/8, [::1] and the
    /// bind host (comma-separated; "*" disables the Host check)
    #[arg(long, env = "VELESDB_ALLOWED_HOSTS", value_delimiter = ',')]
    allowed_hosts: Option<Vec<String>>,
}

/// What the configuration asked for that will not happen, said once the
/// subscriber exists to say it.
fn log_config_warnings(cfg: &ServerConfig, rust_log_overrides_level: bool) {
    if !cfg.ignored_entries.is_empty() {
        tracing::warn!(
            ignored = cfg.ignored_entries.join(", "),
            "these config entries are not read by this velesdb-server build and have \
             no effect; check them against docs/guides/CONFIGURATION.md"
        );
    }
    if rust_log_overrides_level {
        tracing::warn!(
            "RUST_LOG is set and takes precedence over VELESDB_LOGGING_LEVEL and \
             [logging] level; unset it (or set it empty) to use them"
        );
    }
}

fn log_startup(cfg: &ServerConfig) {
    tracing::info!("Starting VelesDB server...");
    tracing::info!(
        "Tokio runtime started with {} worker thread(s)",
        tokio::runtime::Handle::current().metrics().num_workers()
    );
    tracing::info!("Data directory: {}", cfg.data_dir);
    tracing::info!("Bind address: {}:{}", cfg.host, cfg.port);
    if cfg.auth_enabled() {
        tracing::info!(
            "API key authentication enabled ({} key(s))",
            cfg.api_keys.len()
        );
    } else if cfg.binds_publicly() {
        tracing::warn!(
            "API key authentication is DISABLED while binding a publicly reachable \
             address ({}). Every endpoint — including /metrics and all data — is open \
             to anyone who can reach this host. Set VELESDB_API_KEYS (and preferably \
             TLS via VELESDB_TLS_CERT/VELESDB_TLS_KEY) before exposing this server, or \
             bind 127.0.0.1 for local development. See docs/guides/SERVER_SECURITY.md.",
            cfg.host
        );
    } else {
        tracing::info!(
            "API key authentication disabled (local dev mode, bound to {})",
            cfg.host
        );
    }
    if cfg.tls_enabled() {
        tracing::info!("TLS enabled");
    }
    if cfg.rate_limit_enabled() {
        tracing::info!("Rate limiting enabled: {} req/s per IP", cfg.rate_limit);
    } else {
        tracing::info!("Rate limiting disabled");
    }
    log_cors_config(&cfg.cors);
    log_host_policy(cfg);
}

/// Says when the `Host` check will refuse clients that a wildcard bind lets
/// in: the usual cause of a `421` in a compose or Kubernetes deployment.
fn log_host_policy(cfg: &ServerConfig) {
    if cfg.allowed_hosts.iter().any(|h| h == "*") {
        tracing::warn!(
            "allowed_hosts = [\"*\"]: the Host check is off, so a DNS-rebinding page can \
             reach this server as if it were same-origin. List the names clients use \
             instead (see docs/guides/SERVER_SECURITY.md)."
        );
    }
    if cfg.answers_only_loopback_names_on_a_wildcard_bind() {
        tracing::warn!(
            "Bound to {} with no allowed_hosts: only requests addressed to localhost, \
             127.0.0.0/8 or [::1] are answered, any other Host gets 421. List the names \
             clients use (a compose service name, a DNS name, a LAN address) in \
             [server] allowed_hosts, --allowed-hosts or VELESDB_ALLOWED_HOSTS.",
            cfg.host
        );
    }
}

fn log_cors_config(cors: &CorsConfig) {
    if !cors.enabled {
        tracing::info!(
            "CORS disabled ([server] cors_enabled = false): no CORS headers are sent, \
             so browsers refuse cross-origin reads; CORS origin, method and header \
             settings are not applied"
        );
    } else if cors.allowed_origins.is_empty() {
        tracing::info!(
            "CORS: no origin configured, so no CORS header is sent and only \
             same-origin pages can read responses. List a browser front end's \
             origin in [server] cors_origins."
        );
    } else if cors.is_permissive() {
        tracing::warn!(
            "CORS is permissive (cors_origins = [\"*\"]: any web page can read \
             responses). List exact origins in [server] cors_origins for production."
        );
    } else {
        tracing::info!(
            "CORS restricted to {} origin(s)",
            cors.allowed_origins.len()
        );
    }
}

fn init_app_state(
    data_dir: &str,
    core_config: velesdb_core::config::VelesConfig,
) -> anyhow::Result<Arc<AppState>> {
    let db = Database::open_with_config(data_dir, core_config)?;
    let state = Arc::new(AppState {
        db,
        onboarding_metrics: OnboardingMetrics::default(),
        ready: std::sync::atomic::AtomicBool::new(false),
        operational_metrics: velesdb_core::metrics::OperationalMetrics::new_arc(),
        query_duration_histogram: std::sync::Arc::new(
            velesdb_core::metrics::DurationHistogram::new(),
        ),
    });
    // Database loaded successfully — mark server as ready
    state
        .ready
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(state)
}

/// Middleware that adds deprecation headers to responses served on
/// unversioned (legacy) routes. Clients should migrate to `/v1/` prefix.
async fn deprecation_header(
    request: axum::extract::Request,
    next: Next,
) -> axum::response::Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert("deprecation", HeaderValue::from_static("true"));
    headers.insert(
        "x-api-deprecated",
        HeaderValue::from_static("Use /v1/ prefix"),
    );
    response
}

#[allow(clippy::similar_names)] // Reason: `routes` (handler tree) and `router` (final router) are distinct concepts.
fn build_router(
    state: Arc<AppState>,
    auth_state: AuthState,
    cfg: &ServerConfig,
) -> anyhow::Result<Router> {
    let routes = api_routes_with_body_limit(cfg.max_body_size);

    // Canonical versioned API under /v1/
    let versioned = Router::new().nest("/v1", routes.clone());

    // Legacy unversioned routes with deprecation headers for backward compat
    let legacy = routes.layer(axum::middleware::from_fn(deprecation_header));

    let api_router = versioned.merge(legacy).with_state(state);

    #[cfg(feature = "swagger-ui")]
    let api_router = {
        let swagger_ui =
            SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi());
        api_router.merge(Router::<()>::new().merge(swagger_ui))
    };

    let router = api_router.layer(axum::middleware::from_fn_with_state(
        auth_state,
        auth_middleware,
    ));
    let router = apply_browser_policy(router, cfg).layer(TraceLayer::new_for_http());

    if cfg.rate_limit > 0 {
        let config = velesdb_server::rate_limit::build_rate_limit_config(cfg.rate_limit)?;
        Ok(router.layer(velesdb_server::rate_limit::GovernorLayer::new(config)))
    } else {
        Ok(router)
    }
}

/// Returns a future that resolves when SIGTERM is received (Unix) or never (non-Unix).
async fn sigterm() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}

/// Returns a future that resolves when SIGINT (Ctrl+C) or SIGTERM is received.
async fn shutdown_signal() {
    tokio::select! {
        _ = tokio::signal::ctrl_c() => tracing::info!("Received SIGINT (Ctrl+C), initiating graceful shutdown..."),
        () = sigterm() => tracing::info!("Received SIGTERM, initiating graceful shutdown..."),
    }
}

async fn serve(
    host: &str,
    port: u16,
    app: Router,
    state: Arc<AppState>,
    shutdown_timeout_secs: u64,
) -> anyhow::Result<()> {
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("VelesDB server listening on http://{}", addr);

    // Create a notify to track when the shutdown signal fires
    let shutdown_notify = Arc::new(tokio::sync::Notify::new());
    let notify_clone = shutdown_notify.clone();

    let graceful_shutdown = async move {
        shutdown_signal().await;
        notify_clone.notify_one();
    };

    // Every accepted connection is counted in `velesdb_active_connections`.
    // `into_make_service_with_connect_info` provides the peer IP to the rate
    // limiter's SmartIpKeyExtractor; axum implements that for a bare
    // `TcpListener` and for any listener behind `tap_io`, so the counting
    // listener goes through an identity tap to keep it.
    let listener = CountingListener::new(listener, Arc::clone(&state.operational_metrics))
        .tap_io(|_: &mut CountedStream| ());
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(graceful_shutdown)
    .into_future();

    // Start server in a task so we can apply drain timeout after signal
    let server_handle = tokio::spawn(server);

    // Wait for the shutdown signal
    shutdown_notify.notified().await;

    // Now apply drain timeout
    match tokio::time::timeout(
        tokio::time::Duration::from_secs(shutdown_timeout_secs),
        server_handle,
    )
    .await
    {
        Ok(Ok(Ok(()))) => tracing::info!("All connections drained"),
        Ok(Ok(Err(e))) => tracing::warn!("Server error during drain: {e}"),
        Ok(Err(e)) => tracing::warn!("Server task error: {e}"),
        Err(_) => {
            tracing::warn!("Drain timeout ({shutdown_timeout_secs}s) reached, forcing shutdown");
        }
    }

    flush_and_exit(&state);
    Ok(())
}

/// Accepts TLS connections until a shutdown signal is received.
async fn tls_accept_loop(
    listener: tokio::net::TcpListener,
    tls_acceptor: TlsAcceptor,
    app: Router,
    active_conns: Arc<std::sync::atomic::AtomicUsize>,
    metrics: Arc<OperationalMetrics>,
) {
    let shutdown = tokio::signal::ctrl_c();
    let terminate = sigterm();

    tokio::pin!(shutdown);
    tokio::pin!(terminate);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, peer_addr)) => {
                        let open = OpenConnection::new(Arc::clone(&metrics));
                        spawn_tls_connection(stream, peer_addr, tls_acceptor.clone(), app.clone(), active_conns.clone(), open);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to accept TCP connection: {e}");
                    }
                }
            }
            _ = &mut shutdown => {
                tracing::info!("Received SIGINT (Ctrl+C), initiating graceful shutdown...");
                break;
            }
            () = &mut terminate => {
                tracing::info!("Received SIGTERM, initiating graceful shutdown...");
                break;
            }
        }
    }
}

fn spawn_tls_connection(
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    acceptor: TlsAcceptor,
    app: Router,
    conns: Arc<std::sync::atomic::AtomicUsize>,
    open: OpenConnection,
) {
    conns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tokio::spawn(async move {
        // Counted in `velesdb_active_connections` until this task ends.
        let _open = open;
        let Ok(tls_stream) = acceptor.accept(stream).await else {
            tracing::debug!("TLS handshake failed");
            conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            return;
        };

        let io = hyper_util::rt::TokioIo::new(tls_stream);
        let hyper_service = hyper::service::service_fn(move |mut request| {
            let clone = app.clone();
            // The rate limiter's `SmartIpKeyExtractor` reads `ConnectInfo<SocketAddr>`
            // from request extensions. The plaintext path supplies it via
            // `into_make_service_with_connect_info`, but this manual hyper TLS
            // path builds the service by hand, so it must insert `ConnectInfo`
            // itself. Without it every HTTPS request that carries no
            // `X-Forwarded-For`/`X-Real-Ip`/`Forwarded` header (i.e. every
            // normal REST client) failed key extraction and got a blanket 500,
            // making TLS + rate limiting — both on by default in production —
            // unusable together.
            request.extensions_mut().insert(ConnectInfo(peer_addr));
            async move { clone.oneshot(request).await }
        });

        if let Err(err) =
            hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                .serve_connection_with_upgrades(io, hyper_service)
                .await
        {
            tracing::debug!("TLS connection error: {err}");
        }

        conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    });
}

async fn serve_tls(
    host: &str,
    port: u16,
    app: Router,
    cert_path: &str,
    key_path: &str,
    state: Arc<AppState>,
    shutdown_timeout_secs: u64,
) -> anyhow::Result<()> {
    let tls_acceptor = velesdb_server::tls::load_tls_config(cert_path, key_path)?;
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("VelesDB server listening on https://{}", addr);

    let active_conns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    tls_accept_loop(
        listener,
        tls_acceptor,
        app,
        active_conns.clone(),
        Arc::clone(&state.operational_metrics),
    )
    .await;

    drain_connections(&active_conns, shutdown_timeout_secs).await;
    flush_and_exit(&state);
    Ok(())
}

/// Waits for active connections to complete, up to the drain timeout.
async fn drain_connections(active_conns: &std::sync::atomic::AtomicUsize, timeout_secs: u64) {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(timeout_secs);

    loop {
        let count = active_conns.load(std::sync::atomic::Ordering::Relaxed);
        if count == 0 {
            tracing::info!("All active connections drained");
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!(
                "Drain timeout ({timeout_secs}s) reached with {count} active connection(s)"
            );
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

/// Flushes all WALs and logs shutdown completion.
fn flush_and_exit(state: &AppState) {
    tracing::info!("Flushing all WALs...");
    let failures = state.db.flush_all();
    if failures > 0 {
        tracing::warn!("WAL flush completed with {failures} failure(s)");
    } else {
        tracing::info!("All WALs flushed successfully");
    }
    tracing::info!("Shutdown complete");
}

fn build_cli_overrides(args: Args) -> CliOverrides {
    CliOverrides {
        config_path: args.config,
        host: args.host,
        port: args.port,
        data_dir: args.data_dir,
        api_keys: parse_api_keys_env(),
        tls_cert: args.tls_cert,
        tls_key: args.tls_key,
        rate_limit: args.rate_limit,
        log_level: velesdb_server::logging::level_from_env(),
        allowed_hosts: velesdb_server::config::normalize_list_arg(args.allowed_hosts),
    }
}

/// Reads and validates the configuration, installs logging as it says, then
/// starts the runtime it sizes. Both need the configuration first, which is
/// why this is not `#[tokio::main]`: that runtime and its worker count would
/// exist before the file was read.
fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config_path = args.config.clone();
    let cfg = ServerConfig::load(build_cli_overrides(args))?;
    cfg.validate()?;
    let rust_log_overrides_level = velesdb_server::logging::init(&cfg.logging)?;
    log_config_warnings(&cfg, rust_log_overrides_level);

    velesdb_server::runtime::build(cfg.workers)?.block_on(run(cfg, config_path))
}

async fn run(cfg: ServerConfig, config_path: Option<PathBuf>) -> anyhow::Result<()> {
    // Fail-fast: an explicit `--config`/`VELESDB_CONFIG` path that is
    // missing or fails validation aborts startup here with the typed core
    // `ConfigError` — never a silent fallback to engine defaults.
    let core_config = load_core_config(&config_path)?;

    log_startup(&cfg);

    // Non-blocking update check (background thread, 2s timeout), repeated
    // every 24h while the server runs so an install's lifetime is measurable.
    // Disable: VELESDB_NO_UPDATE_CHECK=1 or [update_check] enabled=false in config.
    #[cfg(feature = "update-check")]
    {
        let (check_config, check_context) =
            velesdb_server::config::update_check_plan(config_path.as_deref(), &cfg.data_dir);
        velesdb_core::spawn_update_heartbeat(check_config, check_context);
    }

    let state = init_app_state(&cfg.data_dir, core_config)?;
    let auth_state = AuthState::new(cfg.api_keys.clone());
    let app = build_router(state.clone(), auth_state, &cfg)?;

    if let (Some(cert), Some(key)) = (&cfg.tls.cert, &cfg.tls.key) {
        serve_tls(
            &cfg.host,
            cfg.port,
            app,
            cert,
            key,
            state,
            cfg.shutdown_timeout_secs,
        )
        .await
    } else {
        serve(&cfg.host, cfg.port, app, state, cfg.shutdown_timeout_secs).await
    }
}

#[cfg(all(test, feature = "swagger-ui"))]
#[path = "swagger_ui_tests.rs"]
mod swagger_ui_tests;
