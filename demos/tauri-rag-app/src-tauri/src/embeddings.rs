//! Embedding service using fastembed for semantic search.
//!
//! This module provides real ML-based embeddings using the AllMiniLML6V2 model,
//! which produces 384-dimensional vectors suitable for semantic search.

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::OnceLock;
use tokio::sync::Mutex;

/// Global embedding model instance (lazy-loaded)
static EMBEDDING_MODEL: OnceLock<Mutex<TextEmbedding>> = OnceLock::new();

/// Embedding dimension for AllMiniLML6V2
pub const EMBEDDING_DIM: usize = 384;

/// Model loading status for UI feedback
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelStatus {
    pub loaded: bool,
    pub model_name: String,
    pub dimension: usize,
}

/// Initialize the embedding model (downloads on first use ~90MB)
fn init_model() -> Result<TextEmbedding, String> {
    TextEmbedding::try_new(
        InitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_show_download_progress(true)
    )
    .map_err(|e| format!("Failed to load embedding model: {e}"))
}

/// Get or initialize the global embedding model
pub async fn get_model() -> Result<&'static Mutex<TextEmbedding>, String> {
    // Try to get existing model
    if let Some(model) = EMBEDDING_MODEL.get() {
        return Ok(model);
    }
    
    // Initialize model (blocking operation, but only once)
    let model = tokio::task::spawn_blocking(init_model)
        .await
        .map_err(|e| format!("Task join error: {e}"))??;
    
    // Store in global (ignore if another thread beat us)
    let _ = EMBEDDING_MODEL.set(Mutex::new(model));
    
    EMBEDDING_MODEL
        .get()
        .ok_or_else(|| "Failed to store model".to_string())
}

/// Generate embedding for a single text
pub async fn embed_text(text: &str) -> Result<Vec<f32>, String> {
    let model_mutex = get_model().await?;
    let model = model_mutex.lock().await;
    
    let embeddings = model
        .embed(vec![text], None)
        .map_err(|e| format!("Embedding error: {e}"))?;
    
    embeddings
        .into_iter()
        .next()
        .ok_or_else(|| "No embedding returned".to_string())
}

/// Generate embeddings for multiple texts (batched for efficiency)
pub async fn embed_batch(texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    
    let model_mutex = get_model().await?;
    let model = model_mutex.lock().await;
    
    // Convert to &str for fastembed
    let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
    
    model
        .embed(text_refs, None)
        .map_err(|e| format!("Batch embedding error: {e}"))
}

/// Get model status (for UI)
pub async fn get_status() -> ModelStatus {
    let loaded = EMBEDDING_MODEL.get().is_some();
    ModelStatus {
        loaded,
        model_name: "AllMiniLML6V2".to_string(),
        dimension: EMBEDDING_DIM,
    }
}

/// Preload the model (call at startup for better UX)
pub async fn preload() -> Result<(), String> {
    get_model().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires model download
    async fn test_embed_text() {
        let embedding = embed_text("Hello, world!").await.unwrap();
        assert_eq!(embedding.len(), EMBEDDING_DIM);
    }

    #[tokio::test]
    #[ignore] // Requires model download
    async fn test_embed_batch() {
        let texts = vec![
            "Hello".to_string(),
            "World".to_string(),
        ];
        let embeddings = embed_batch(texts).await.unwrap();
        assert_eq!(embeddings.len(), 2);
        assert_eq!(embeddings[0].len(), EMBEDDING_DIM);
    }
}
