//! Tauri commands for RAG operations

use serde::{Deserialize, Serialize};
use tauri_plugin_velesdb::VelesDbExt;

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

/// Generate a simple embedding (placeholder - use a real model in production)
/// 
/// In production, use:
/// - `rust-bert` for local transformers
/// - `candle` for efficient inference
/// - External API (OpenAI, Cohere, etc.)
fn generate_embedding(text: &str) -> Vec<f32> {
    // Simple hash-based embedding for demo purposes
    // Replace with real embedding model in production!
    let mut embedding = vec![0.0_f32; 128];
    
    for (i, c) in text.chars().enumerate() {
        let idx = i % 128;
        embedding[idx] += (c as u32 as f32) / 1000.0;
    }
    
    // Normalize
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut embedding {
            *val /= norm;
        }
    }
    
    embedding
}

/// Ingest text and create embeddings
#[tauri::command]
pub async fn ingest_text(
    app: tauri::AppHandle,
    text: String,
    chunk_size: Option<usize>,
) -> Result<Vec<Chunk>, String> {
    let chunk_size = chunk_size.unwrap_or(500);
    let chunks = chunk_text(&text, chunk_size);
    let mut result = Vec::new();

    let db = app.velesdb();

    for (i, chunk_text) in chunks.iter().enumerate() {
        let id = i as u64;
        let embedding = generate_embedding(chunk_text);

        db.insert(id, &embedding)
            .map_err(|e| format!("Insert error: {e}"))?;

        result.push(Chunk {
            id,
            text: chunk_text.clone(),
            score: None,
        });
    }

    Ok(result)
}

/// Search for similar chunks
#[tauri::command]
pub async fn search(
    app: tauri::AppHandle,
    query: String,
    k: Option<usize>,
) -> Result<SearchResult, String> {
    let start = std::time::Instant::now();
    let k = k.unwrap_or(5);

    let query_embedding = generate_embedding(&query);
    let db = app.velesdb();

    let results = db
        .search(&query_embedding, k)
        .map_err(|e| format!("Search error: {e}"))?;

    let chunks: Vec<Chunk> = results
        .iter()
        .map(|(id, score)| Chunk {
            id: *id,
            text: format!("Chunk {id}"), // In real app, retrieve from storage
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
        dimension: db.dimension(),
    })
}

/// Clear the index
#[tauri::command]
pub async fn clear_index(app: tauri::AppHandle) -> Result<(), String> {
    let db = app.velesdb();
    db.clear();
    Ok(())
}
