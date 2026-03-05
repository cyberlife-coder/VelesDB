//! Tauri commands for RAG operations.
//!
//! Uses the full `VelesDbState` from `tauri-plugin-velesdb` for persistent vector storage.
//! Text chunks are stored directly in each `Point`'s payload — no separate JSON file needed.

use crate::embeddings;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Manager};
use tauri_plugin_velesdb::{Error as PluginError, VelesDbState};
use velesdb_core::{Database, DistanceMetric, Point};

/// Name of the persistent VectorCollection used for RAG.
const COLLECTION: &str = "rag-docs";

/// Monotonic chunk-ID counter.
///
/// Seeded from the persisted collection's max ID on first use (see `ensure_collection`),
/// so IDs never collide across app restarts. Reset to 0 only by `clear_index`.
static NEXT_CHUNK_ID: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    // Relaxed is sufficient: we only need uniqueness, not synchronisation with other data.
    NEXT_CHUNK_ID.fetch_add(1, Ordering::Relaxed)
}

// ─── DTOs ────────────────────────────────────────────────────────────────────

/// A text chunk, returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: u64,
    pub text: String,
    pub score: Option<f32>,
}

/// Full search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunks: Vec<Chunk>,
    pub query: String,
    /// VelesDB vector-search latency only (embedding time excluded).
    pub time_ms: f64,
}

/// Index-level statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_chunks: usize,
    pub dimension: usize,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Split `text` into chunks no larger than `chunk_size` bytes, breaking on paragraph
/// boundaries (`\n\n`).
fn chunk_text(text: &str, chunk_size: usize) -> Vec<String> {
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    for para in paragraphs {
        if current_chunk.len() + para.len() > chunk_size && !current_chunk.is_empty() {
            chunks.push(current_chunk.trim().to_string());
            current_chunk = String::new();
        }
        current_chunk.push_str(para);
        current_chunk.push_str("\n\n");
    }

    if !current_chunk.trim().is_empty() {
        chunks.push(current_chunk.trim().to_string());
    }

    chunks
}

/// Ensure the RAG collection exists, creating it if needed.
///
/// When the collection already exists, the ID counter is seeded from the persisted
/// max ID so that new chunks never overwrite existing ones across restarts.
fn ensure_collection(state: &VelesDbState) -> Result<(), String> {
    state
        .with_db(|db: Arc<Database>| {
            if let Some(coll) = db.get_vector_collection(COLLECTION) {
                // Advance counter past any IDs already on disk to prevent overwrites.
                let persisted_max = coll.all_ids().into_iter().max().unwrap_or(0);
                let needed_next = persisted_max.saturating_add(1);
                let current = NEXT_CHUNK_ID.load(Ordering::Relaxed);
                if needed_next > current {
                    NEXT_CHUNK_ID.store(needed_next, Ordering::Relaxed);
                }
            } else {
                db.create_vector_collection(
                    COLLECTION,
                    embeddings::EMBEDDING_DIM,
                    DistanceMetric::Cosine,
                )
                .map_err(PluginError::Database)?;
            }
            Ok(())
        })
        .map_err(|e| format!("DB error: {e}"))
}

// ─── Tauri commands ───────────────────────────────────────────────────────────

/// Chunk `text`, embed each chunk with the ML model, and persist to VelesDB.
///
/// The raw text is stored inside each `Point`'s JSON payload (`{"text": "…"}`),
/// so no external file storage is needed.
#[tauri::command]
pub async fn ingest_text(
    app: AppHandle,
    text: String,
    chunk_size: Option<usize>,
) -> Result<Vec<Chunk>, String> {
    let chunk_size = chunk_size.unwrap_or(500);
    let chunk_texts = chunk_text(&text, chunk_size);

    if chunk_texts.is_empty() {
        return Ok(vec![]);
    }

    // Embed all chunks in one batch (efficient).
    let embeddings = embeddings::embed_batch(chunk_texts.clone()).await?;

    let state = app.state::<VelesDbState>();
    ensure_collection(&state)?;

    // Build IDs and Points together so we can return the Chunk list after insertion.
    let mut ids = Vec::with_capacity(chunk_texts.len());
    let points: Vec<Point> = chunk_texts
        .iter()
        .zip(embeddings.iter())
        .map(|(chunk, embedding)| {
            let id = next_id();
            ids.push(id);
            let payload = serde_json::json!({ "text": chunk });
            Point::new(id, embedding.clone(), Some(payload))
        })
        .collect();

    state
        .with_db(|db: Arc<Database>| {
            let coll = db
                .get_vector_collection(COLLECTION)
                .ok_or_else(|| PluginError::CollectionNotFound(COLLECTION.to_string()))?;
            coll.upsert_bulk(&points)
                .map(|_| ())
                .map_err(PluginError::Database)
        })
        .map_err(|e| format!("Insert error: {e}"))?;

    let result = chunk_texts
        .into_iter()
        .zip(ids)
        .map(|(text, id)| Chunk { id, text, score: None })
        .collect();

    Ok(result)
}

/// Embed `query`, search the VectorCollection, and return the top-k chunks with scores.
///
/// `time_ms` reflects VelesDB search latency only — embedding is excluded so the
/// frontend shows an accurate picture of the DB's performance.
#[tauri::command]
pub async fn search(
    app: AppHandle,
    query: String,
    k: Option<usize>,
) -> Result<SearchResult, String> {
    let k = k.unwrap_or(5);

    // Embedding runs outside the timer — it's ML inference, not VelesDB latency.
    let query_embedding = embeddings::embed_text(&query).await?;

    let state = app.state::<VelesDbState>();
    ensure_collection(&state)?;

    // Time only the vector search itself.
    let search_start = std::time::Instant::now();
    let raw = state
        .with_db(|db: Arc<Database>| {
            let coll = db
                .get_vector_collection(COLLECTION)
                .ok_or_else(|| PluginError::CollectionNotFound(COLLECTION.to_string()))?;
            coll.search(&query_embedding, k).map_err(PluginError::Database)
        })
        .map_err(|e| format!("Search error: {e}"))?;
    let search_ms = search_start.elapsed().as_secs_f64() * 1000.0;

    let chunks = raw
        .into_iter()
        .map(|sr| {
            // Text was stored in the payload at ingest time.
            let text = sr
                .point
                .payload
                .as_ref()
                .and_then(|p| p.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Chunk {
                id: sr.point.id,
                text,
                score: Some(sr.score),
            }
        })
        .collect();

    Ok(SearchResult {
        chunks,
        query,
        time_ms: search_ms,
    })
}

/// Return the number of indexed chunks and the vector dimension.
#[tauri::command]
pub async fn get_stats(app: AppHandle) -> Result<IndexStats, String> {
    let state = app.state::<VelesDbState>();
    let total_chunks = state
        .with_db(|db: Arc<Database>| {
            Ok(db
                .get_vector_collection(COLLECTION)
                .map_or(0, |c| c.len()))
        })
        .map_err(|e| format!("Stats error: {e}"))?;

    Ok(IndexStats {
        total_chunks,
        dimension: embeddings::EMBEDDING_DIM,
    })
}

/// Drop and recreate the RAG collection, clearing all indexed vectors.
#[tauri::command]
pub async fn clear_index(app: AppHandle) -> Result<(), String> {
    let state = app.state::<VelesDbState>();
    state
        .with_db(|db: Arc<Database>| {
            if db.get_vector_collection(COLLECTION).is_some() {
                db.delete_collection(COLLECTION)
                    .map_err(PluginError::Database)?;
            }
            db.create_vector_collection(
                COLLECTION,
                embeddings::EMBEDDING_DIM,
                DistanceMetric::Cosine,
            )
            .map_err(PluginError::Database)
        })
        .map_err(|e| format!("Clear error: {e}"))?;

    NEXT_CHUNK_ID.store(0, Ordering::Relaxed);
    Ok(())
}

/// Return the current embedding model status (loaded / model name / dimension).
#[tauri::command]
pub async fn get_model_status() -> Result<embeddings::ModelStatus, String> {
    Ok(embeddings::get_status().await)
}

/// Trigger model download/initialisation at startup for better UX.
#[tauri::command]
pub async fn preload_model() -> Result<(), String> {
    embeddings::preload().await
}
