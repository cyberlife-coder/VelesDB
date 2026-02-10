#![allow(clippy::doc_markdown)]
//! `VelesDB` Server - REST API for the `VelesDB` vector database.

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use clap::Parser;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use velesdb_core::Database;
use velesdb_server::{
    add_edge, batch_search, create_collection, create_index, delete_collection, delete_index,
    delete_point, explain, flush_collection, get_collection, get_edges, get_node_degree, get_point,
    health_check, hybrid_search, is_empty, list_collections, list_indexes, match_query,
    multi_query_search, query, search, stream_traverse, text_search, traverse_graph, upsert_points,
    ApiDoc, AppState,
};

/// VelesDB Server - A high-performance vector database
#[derive(Parser, Debug)]
#[command(name = "velesdb-server")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Data directory for persistent storage
    #[arg(short, long, default_value = "./data", env = "VELESDB_DATA_DIR")]
    data_dir: String,

    /// Host address to bind to
    #[arg(long, default_value = "0.0.0.0", env = "VELESDB_HOST")]
    host: String,

    /// Port to listen on
    #[arg(short, long, default_value = "8080", env = "VELESDB_PORT")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info,tower_http=debug".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Parse command line arguments
    let args = Args::parse();

    tracing::info!("Starting VelesDB server...");
    tracing::info!("Data directory: {}", args.data_dir);

    // Initialize database
    let db = Database::open(&args.data_dir)?;
    let state = Arc::new(AppState { db });

    tracing::info!(
        "Graph EdgeStore is in-memory (shared with Collection). \
         Edge data will NOT persist across restarts. Disk persistence planned for future release."
    );

    // Build API router — all routes share AppState (graph included)
    let api_router = Router::new()
        .route("/health", get(health_check))
        .route(
            "/collections",
            get(list_collections).post(create_collection),
        )
        .route(
            "/collections/{name}",
            get(get_collection).delete(delete_collection),
        )
        .route("/collections/{name}/empty", get(is_empty))
        .route("/collections/{name}/flush", post(flush_collection))
        // 100MB limit for batch vector uploads (1000 vectors × 768D × 4 bytes = ~3MB typical)
        .route("/collections/{name}/points", post(upsert_points))
        .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
        .route(
            "/collections/{name}/points/{id}",
            get(get_point).delete(delete_point),
        )
        .route("/collections/{name}/search", post(search))
        .route("/collections/{name}/search/batch", post(batch_search))
        .route("/collections/{name}/search/multi", post(multi_query_search))
        .route("/collections/{name}/search/text", post(text_search))
        .route("/collections/{name}/search/hybrid", post(hybrid_search))
        .route(
            "/collections/{name}/indexes",
            get(list_indexes).post(create_index),
        )
        .route(
            "/collections/{name}/indexes/{label}/{property}",
            delete(delete_index),
        )
        .route("/query", post(query))
        .route("/query/explain", post(explain))
        .route("/collections/{name}/match", post(match_query))
        // Graph routes — delegate to Collection methods from velesdb-core
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
        .with_state(state);

    // FLAG-3 FIX: Add metrics endpoint conditionally (EPIC-016/US-034,035)
    #[cfg(feature = "prometheus")]
    let api_router = {
        use velesdb_server::prometheus_metrics;
        api_router.route("/metrics", get(prometheus_metrics))
    };

    // Swagger UI (stateless router)
    let swagger_ui = SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi());

    // Build main app with Swagger UI
    let app = api_router
        .merge(Router::<()>::new().merge(swagger_ui))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // Start server
    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("VelesDB server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}
