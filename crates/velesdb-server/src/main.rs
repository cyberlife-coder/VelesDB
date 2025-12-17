//! VelesDB Server - REST API for the VelesDB vector database.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use velesdb_core::{Database, DistanceMetric, Point};

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

/// Application state shared across handlers.
struct AppState {
    db: Database,
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

    // Build router
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/collections", get(list_collections).post(create_collection))
        .route(
            "/collections/:name",
            get(get_collection).delete(delete_collection),
        )
        .route("/collections/:name/points", post(upsert_points))
        .route("/collections/:name/points/:id", get(get_point).delete(delete_point))
        .route("/collections/:name/search", post(search))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // Start server
    let addr = format!("{}:{}", args.host, args.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    tracing::info!("VelesDB server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

// ============================================================================
// Request/Response Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct CreateCollectionRequest {
    name: String,
    dimension: usize,
    #[serde(default = "default_metric")]
    metric: String,
}

fn default_metric() -> String {
    "cosine".to_string()
}

#[derive(Debug, Serialize)]
struct CollectionResponse {
    name: String,
    dimension: usize,
    metric: String,
    point_count: usize,
}

#[derive(Debug, Deserialize)]
struct UpsertPointsRequest {
    points: Vec<PointRequest>,
}

#[derive(Debug, Deserialize)]
struct PointRequest {
    id: u64,
    vector: Vec<f32>,
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    vector: Vec<f32>,
    #[serde(default = "default_top_k")]
    top_k: usize,
}

fn default_top_k() -> usize {
    10
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    results: Vec<SearchResultResponse>,
}

#[derive(Debug, Serialize)]
struct SearchResultResponse {
    id: u64,
    score: f32,
    payload: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

// ============================================================================
// Handlers
// ============================================================================

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "healthy",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn list_collections(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let collections = state.db.list_collections();
    Json(serde_json::json!({ "collections": collections }))
}

async fn create_collection(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateCollectionRequest>,
) -> impl IntoResponse {
    let metric = match req.metric.to_lowercase().as_str() {
        "cosine" => DistanceMetric::Cosine,
        "euclidean" | "l2" => DistanceMetric::Euclidean,
        "dot" | "dotproduct" | "ip" => DistanceMetric::DotProduct,
        _ => return (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: format!("Invalid metric: {}", req.metric),
        })).into_response(),
    };

    match state.db.create_collection(&req.name, req.dimension, metric) {
        Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({
            "message": "Collection created",
            "name": req.name
        }))).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: e.to_string(),
        })).into_response(),
    }
}

async fn get_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.get_collection(&name) {
        Some(collection) => {
            let config = collection.config();
            Json(CollectionResponse {
                name: config.name,
                dimension: config.dimension,
                metric: format!("{:?}", config.metric).to_lowercase(),
                point_count: config.point_count,
            }).into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: format!("Collection '{}' not found", name),
        })).into_response(),
    }
}

async fn delete_collection(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.db.delete_collection(&name) {
        Ok(()) => Json(serde_json::json!({
            "message": "Collection deleted",
            "name": name
        })).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: e.to_string(),
        })).into_response(),
    }
}

async fn upsert_points(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<UpsertPointsRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: format!("Collection '{}' not found", name),
        })).into_response(),
    };

    let points: Vec<Point> = req
        .points
        .into_iter()
        .map(|p| Point::new(p.id, p.vector, p.payload))
        .collect();

    let count = points.len();

    match collection.upsert(points) {
        Ok(()) => Json(serde_json::json!({
            "message": "Points upserted",
            "count": count
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: e.to_string(),
        })).into_response(),
    }
}

async fn get_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: format!("Collection '{}' not found", name),
        })).into_response(),
    };

    let points = collection.get(&[id]);

    match points.into_iter().next().flatten() {
        Some(point) => Json(serde_json::json!({
            "id": point.id,
            "vector": point.vector,
            "payload": point.payload
        })).into_response(),
        None => (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: format!("Point {} not found", id),
        })).into_response(),
    }
}

async fn delete_point(
    State(state): State<Arc<AppState>>,
    Path((name, id)): Path<(String, u64)>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: format!("Collection '{}' not found", name),
        })).into_response(),
    };

    match collection.delete(&[id]) {
        Ok(()) => Json(serde_json::json!({
            "message": "Point deleted",
            "id": id
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: e.to_string(),
        })).into_response(),
    }
}

async fn search(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(req): Json<SearchRequest>,
) -> impl IntoResponse {
    let collection = match state.db.get_collection(&name) {
        Some(c) => c,
        None => return (StatusCode::NOT_FOUND, Json(ErrorResponse {
            error: format!("Collection '{}' not found", name),
        })).into_response(),
    };

    match collection.search(&req.vector, req.top_k) {
        Ok(results) => {
            let response = SearchResponse {
                results: results
                    .into_iter()
                    .map(|r| SearchResultResponse {
                        id: r.point.id,
                        score: r.score,
                        payload: r.point.payload,
                    })
                    .collect(),
            };
            Json(response).into_response()
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(ErrorResponse {
            error: e.to_string(),
        })).into_response(),
    }
}
