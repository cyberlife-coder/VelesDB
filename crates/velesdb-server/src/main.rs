#![allow(clippy::doc_markdown)]
//! `VelesDB` Server - REST API for the `VelesDB` vector database.

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
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
    add_edge, aggregate, analyze_collection,
    auth::{auth_middleware, AuthState},
    batch_search, collection_sanity,
    config::{parse_api_keys_env, CliOverrides, ServerConfig},
    create_collection, create_index, delete_collection, delete_index, delete_point, explain,
    flush_collection, get_collection, get_collection_config, get_collection_stats, get_edges,
    get_guardrails, get_node_degree, get_point, health_check, hybrid_search, is_empty,
    list_collections, list_indexes, match_query, multi_query_search, query, readiness_check,
    search, search_ids, stream_insert, stream_traverse, stream_upsert_points, text_search,
    traverse_graph, update_guardrails, upsert_points, AppState, OnboardingMetrics,
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
}

fn init_app_state(data_dir: &str) -> anyhow::Result<Arc<AppState>> {
    let db = Database::open(data_dir)?;
    let state = Arc::new(AppState {
        db,
        onboarding_metrics: OnboardingMetrics::default(),
        query_limits: parking_lot::RwLock::new(velesdb_core::guardrails::QueryLimits::default()),
        ready: std::sync::atomic::AtomicBool::new(false),
    });
    // Database loaded successfully — mark server as ready
    state
        .ready
        .store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(state)
}

fn core_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(readiness_check))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/collections/{name}",
            get(get_collection).delete(delete_collection),
        )
        .route("/collections/{name}/empty", get(is_empty))
        .route("/collections/{name}/config", get(get_collection_config))
        .route("/collections/{name}/sanity", get(collection_sanity))
        .route("/collections/{name}/flush", post(flush_collection))
        .route("/collections/{name}/analyze", post(analyze_collection))
        .route("/collections/{name}/stats", get(get_collection_stats))
        .route("/guardrails", get(get_guardrails).put(update_guardrails))
        // 100MB limit scoped to batch vector upload routes only
        // (1000 vectors × 768D × 4 bytes = ~3MB typical; 100MB covers extreme cases)
        .merge(
            Router::new()
                .route("/collections/{name}/points", post(upsert_points))
                .route(
                    "/collections/{name}/points/stream",
                    post(stream_upsert_points),
                )
                .layer(DefaultBodyLimit::max(100 * 1024 * 1024)),
        )
        .route("/collections/{name}/stream/insert", post(stream_insert))
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
}

fn search_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .route("/collections/{name}/search/multi", post(multi_query_search))
        .route("/collections/{name}/search/text", post(text_search))
        .route("/collections/{name}/search/hybrid", post(hybrid_search))
        .route("/collections/{name}/search/ids", post(search_ids))
        .route(
            "/collections/{name}/indexes",
            get(list_indexes).post(create_index),
        )
        .route(
            "/collections/{name}/indexes/{label}/{property}",
            delete(delete_index),
        )
        .route("/query", post(query))
        .route("/aggregate", post(aggregate))
        .route("/query/explain", post(explain))
        .route("/collections/{name}/match", post(match_query))
}

fn graph_routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/collections/{name}/graph/edges",
            get(get_edges).post(add_edge),
        )
        .route("/collections/{name}/graph/traverse", post(traverse_graph))
        .route(
            "/collections/{name}/graph/traverse/stream",
            get(stream_traverse),
        )
        .route(
            "/collections/{name}/graph/nodes/{node_id}/degree",
            get(get_node_degree),
        )
}

fn api_routes() -> Router<Arc<AppState>> {
    core_routes().merge(search_routes()).merge(graph_routes())
}

fn build_router(state: Arc<AppState>, auth_state: AuthState) -> Router {
    let api_router = api_routes();

    #[cfg(feature = "prometheus")]
    let api_router = {
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

    api_router
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            auth_middleware,
        ))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
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

    let server = axum::serve(listener, app)
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
    let app = build_router(state.clone(), auth_state);

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
