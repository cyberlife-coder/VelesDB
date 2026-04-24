#![allow(clippy::doc_markdown)]
//! `VelesDB` Server - REST API for the `VelesDB` vector database.

use axum::{middleware::Next, Router};
use clap::Parser;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[cfg(feature = "swagger-ui")]
use utoipa::OpenApi;
#[cfg(feature = "swagger-ui")]
use utoipa_swagger_ui::SwaggerUi;
use velesdb_core::Database;
#[cfg(feature = "swagger-ui")]
use velesdb_server::ApiDoc;
use velesdb_server::{
    auth::{auth_middleware, AuthState},
    config::{build_cors_layer, parse_api_keys_env, CliOverrides, CorsConfig, ServerConfig},
    routes::api_routes,
    AppState, OnboardingMetrics,
};

/// VelesDB Server - A high-performance vector database
#[derive(Parser, Debug)]
#[command(name = "velesdb-server")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to velesdb.toml configuration file
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
}

fn configure_tracing() {
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .try_init();
}

fn log_startup(cfg: &ServerConfig) {
    tracing::info!("Starting VelesDB server...");
    tracing::info!("Data directory: {}", cfg.data_dir);
    tracing::info!("Bind address: {}:{}", cfg.host, cfg.port);
    if cfg.auth_enabled() {
        tracing::info!(
            "API key authentication enabled ({} key(s))",
            cfg.api_keys.len()
        );
    } else {
        tracing::info!("API key authentication disabled (local dev mode)");
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
}

fn log_cors_config(cors: &CorsConfig) {
    if cors.is_permissive() {
        tracing::warn!(
            "CORS is permissive (all origins allowed). \
             Set [cors] allowed_origins in velesdb.toml for production."
        );
    } else {
        tracing::info!(
            "CORS restricted to {} origin(s)",
            cors.allowed_origins.len()
        );
    }
}

fn init_app_state(data_dir: &str) -> anyhow::Result<Arc<AppState>> {
    let db = Database::open(data_dir)?;
    let state = Arc::new(AppState {
        db,
        onboarding_metrics: OnboardingMetrics::default(),
        query_limits: parking_lot::RwLock::new(velesdb_core::guardrails::QueryLimits::default()),
        ready: std::sync::atomic::AtomicBool::new(false),
        operational_metrics: velesdb_core::metrics::OperationalMetrics::shared(),
        traversal_metrics: std::sync::Arc::new(velesdb_core::metrics::TraversalMetrics::new()),
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
    headers.insert("deprecation", "true".parse().expect("static header value"));
    headers.insert(
        "x-api-deprecated",
        "Use /v1/ prefix".parse().expect("static header value"),
    );
    response
}

#[allow(clippy::similar_names)] // Reason: `routes` (handler tree) and `router` (final router) are distinct concepts.
fn build_router(
    state: Arc<AppState>,
    auth_state: AuthState,
    rate_limit: u32,
    cors: &CorsConfig,
) -> anyhow::Result<Router> {
    let routes = api_routes();

    // Canonical versioned API under /v1/
    let versioned = Router::new().nest("/v1", routes.clone());

    // Legacy unversioned routes with deprecation headers for backward compat
    let legacy = routes.layer(axum::middleware::from_fn(deprecation_header));

    let api_router = versioned.merge(legacy);

    #[cfg(feature = "prometheus")]
    let api_router = {
        use axum::routing::get;
        use velesdb_server::prometheus_metrics;
        api_router.route("/metrics", get(prometheus_metrics))
    };

    let api_router = api_router.with_state(state);

    #[cfg(feature = "swagger-ui")]
    let api_router = {
        let swagger_ui =
            SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi());
        api_router.merge(Router::<()>::new().merge(swagger_ui))
    };

    let cors_layer = build_cors_layer(cors);

    let router = api_router
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            auth_middleware,
        ))
        .layer(cors_layer)
        .layer(TraceLayer::new_for_http());

    if rate_limit > 0 {
        let config = velesdb_server::rate_limit::build_rate_limit_config(rate_limit)?;
        Ok(router.layer(velesdb_server::rate_limit::GovernorLayer::new(config)))
    } else {
        Ok(router)
    }
}

fn warn_if_exposed(host: &str) {
    if host != "127.0.0.1" && host != "localhost" {
        tracing::warn!(
            "VelesDB server exposed on network ({host}). \
             Consider using 127.0.0.1 for local-first usage."
        );
    }
}

/// Returns a future that resolves when SIGTERM is received (Unix) or never (non-Unix).
async fn sigterm() {
    #[cfg(unix)]
    {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
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
    warn_if_exposed(host);
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

    // into_make_service_with_connect_info provides peer IP to the
    // rate limiter's SmartIpKeyExtractor.
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
) {
    let shutdown = tokio::signal::ctrl_c();
    let terminate = sigterm();

    tokio::pin!(shutdown);
    tokio::pin!(terminate);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _peer_addr)) => {
                        spawn_tls_connection(stream, tls_acceptor.clone(), app.clone(), active_conns.clone());
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
    acceptor: TlsAcceptor,
    app: Router,
    conns: Arc<std::sync::atomic::AtomicUsize>,
) {
    conns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    tokio::spawn(async move {
        let Ok(tls_stream) = acceptor.accept(stream).await else {
            tracing::debug!("TLS handshake failed");
            conns.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            return;
        };

        let io = hyper_util::rt::TokioIo::new(tls_stream);
        let hyper_service = hyper::service::service_fn(move |request| {
            let clone = app.clone();
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
    warn_if_exposed(host);

    let tls_acceptor = velesdb_server::tls::load_tls_config(cert_path, key_path)?;
    let addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("VelesDB server listening on https://{}", addr);

    let active_conns = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    tls_accept_loop(listener, tls_acceptor, app, active_conns.clone()).await;

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
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    configure_tracing();

    let args = Args::parse();
    let cli = build_cli_overrides(args);
    let cfg = ServerConfig::load(cli)?;
    cfg.validate()?;

    log_startup(&cfg);

    // Non-blocking update check (background thread, 2s timeout).
    // Disable: VELESDB_NO_UPDATE_CHECK=1 or [update_check] enabled=false in config.
    #[cfg(feature = "update-check")]
    velesdb_core::spawn_update_check(
        velesdb_core::UpdateCheckConfig::default(),
        std::path::PathBuf::from(&cfg.data_dir),
        "core".to_string(),
    );

    let state = init_app_state(&cfg.data_dir)?;
    let auth_state = AuthState::new(cfg.api_keys.clone());
    let app = build_router(state.clone(), auth_state, cfg.rate_limit, &cfg.cors)?;

    if let (Some(cert), Some(key)) = (&cfg.tls_cert, &cfg.tls_key) {
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
