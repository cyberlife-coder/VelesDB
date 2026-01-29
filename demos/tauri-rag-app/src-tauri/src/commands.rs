//! Tauri commands for RAG operations

#[cfg(test)]
mod tests;

use crate::embeddings;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri_plugin_velesdb::VelesDbExt;

/// Counter for unique chunk IDs (persists across ingestions)
static NEXT_CHUNK_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn get_next_chunk_id() -> u64 {
    NEXT_CHUNK_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

fn reset_chunk_id_counter() {
    NEXT_CHUNK_ID.store(0, std::sync::atomic::Ordering::SeqCst);
}

/// Persistent storage for chunk texts
#[derive(Serialize, Deserialize, Default)]
struct ChunkStore {
    chunks: HashMap<u64, String>,
    next_id: u64,
}

/// In-memory storage for chunk texts (synced to disk)
static CHUNK_TEXTS: Mutex<Option<HashMap<u64, String>>> = Mutex::new(None);
static DATA_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn get_data_path() -> PathBuf {
    let guard = DATA_PATH.lock().unwrap_or_else(|p| p.into_inner());
    guard.clone().unwrap_or_else(|| PathBuf::from("./velesdb_data/chunks.json"))
}

fn set_data_path(path: PathBuf) {
    let mut guard = DATA_PATH.lock().unwrap_or_else(|p| p.into_inner());
    *guard = Some(path);
}

fn get_chunk_texts() -> std::sync::MutexGuard<'static, Option<HashMap<u64, String>>> {
    CHUNK_TEXTS.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Load chunks from disk on startup
fn load_chunks_from_disk() -> HashMap<u64, String> {
    let path = get_data_path();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(store) = serde_json::from_str::<ChunkStore>(&content) {
                // Restore the ID counter
                NEXT_CHUNK_ID.store(store.next_id, std::sync::atomic::Ordering::SeqCst);
                return store.chunks;
            }
        }
    }
    HashMap::new()
}

/// Save chunks to disk
fn save_chunks_to_disk() {
    let guard = get_chunk_texts();
    if let Some(chunks) = guard.as_ref() {
        let store = ChunkStore {
            chunks: chunks.clone(),
            next_id: NEXT_CHUNK_ID.load(std::sync::atomic::Ordering::SeqCst),
        };
        
        let path = get_data_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        
        if let Ok(json) = serde_json::to_string_pretty(&store) {
            let _ = fs::write(&path, json);
        }
    }
}

fn store_chunk_text(id: u64, text: String) {
    let mut guard = get_chunk_texts();
    if guard.is_none() {
        *guard = Some(load_chunks_from_disk());
    }
    if let Some(map) = guard.as_mut() {
        map.insert(id, text);
    }
    drop(guard);
    save_chunks_to_disk();
}

fn get_chunk_text(id: u64) -> String {
    let mut guard = get_chunk_texts();
    if guard.is_none() {
        *guard = Some(load_chunks_from_disk());
    }
    guard
        .as_ref()
        .and_then(|map| map.get(&id))
        .cloned()
        .unwrap_or_else(|| format!("Chunk {id}"))
}

fn clear_chunk_texts() {
    let mut guard = get_chunk_texts();
    *guard = Some(HashMap::new());
    drop(guard);
    save_chunks_to_disk();
}

/// Document chunk with text and embedding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: u64,
    pub text: String,
    pub score: Option<f32>,
}

/// Search result from VelesDB
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub chunks: Vec<Chunk>,
    pub query: String,
    pub time_ms: f64,
}

/// Index statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_chunks: usize,
    pub dimension: usize,
}

/// Simple text chunking (split by paragraphs)
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

/// Ingest text and create embeddings using ML model
#[tauri::command]
pub async fn ingest_text(
    app: tauri::AppHandle,
    text: String,
    chunk_size: Option<usize>,
) -> Result<Vec<Chunk>, String> {
    let chunk_size = chunk_size.unwrap_or(500);
    let chunks = chunk_text(&text, chunk_size);
    
    if chunks.is_empty() {
        return Ok(vec![]);
    }
    
    // Generate embeddings for all chunks using ML model
    let embeddings = embeddings::embed_batch(chunks.clone()).await?;
    
    let db = app.velesdb();
    let mut result = Vec::new();

    for (chunk_text, embedding) in chunks.iter().zip(embeddings.iter()) {
        let id = get_next_chunk_id();

        db.insert(id, embedding)
            .map_err(|e| format!("Insert error: {e}"))?;

        store_chunk_text(id, chunk_text.clone());

        result.push(Chunk {
            id,
            text: chunk_text.clone(),
            score: None,
        });
    }

    Ok(result)
}

/// Search for similar chunks using semantic embeddings
#[tauri::command]
pub async fn search(
    app: tauri::AppHandle,
    query: String,
    k: Option<usize>,
) -> Result<SearchResult, String> {
    let start = std::time::Instant::now();
    let k = k.unwrap_or(5);

    // Generate query embedding using ML model
    let query_embedding = embeddings::embed_text(&query).await?;
    let db = app.velesdb();

    let results = db
        .search(&query_embedding, k)
        .map_err(|e| format!("Search error: {e}"))?;

    let chunks: Vec<Chunk> = results
        .iter()
        .map(|(id, score)| Chunk {
            id: *id,
            text: get_chunk_text(*id),  // Retrieve actual text from storage
            score: Some(*score),
        })
        .collect();

    let time_ms = start.elapsed().as_secs_f64() * 1000.0;

    Ok(SearchResult {
        chunks,
        query,
        time_ms,
    })
}

/// Get index statistics
#[tauri::command]
pub async fn get_stats(app: tauri::AppHandle) -> Result<IndexStats, String> {
    let db = app.velesdb();

    Ok(IndexStats {
        total_chunks: db.len(),
        dimension: embeddings::EMBEDDING_DIM,
    })
}

/// Get embedding model status
#[tauri::command]
pub async fn get_model_status() -> Result<embeddings::ModelStatus, String> {
    Ok(embeddings::get_status().await)
}

/// Preload the embedding model (call at startup)
#[tauri::command]
pub async fn preload_model() -> Result<(), String> {
    embeddings::preload().await
}

/// Clear the index
#[tauri::command]
pub async fn clear_index(app: tauri::AppHandle) -> Result<(), String> {
    let db = app.velesdb();
    db.clear();
    clear_chunk_texts();  // Also clear stored texts
    reset_chunk_id_counter();  // Reset ID counter
    Ok(())
}
