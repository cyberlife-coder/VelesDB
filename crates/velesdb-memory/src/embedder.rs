//! Pluggable text → vector embedding.
//!
//! The Agent Memory SDK is *bring-your-own-vector*: it never generates
//! embeddings. This crate mirrors the repo's established pattern (the Python
//! SDK's `Embedder` protocol, the tauri-rag demo's `fastembed` backend): an
//! [`Embedder`] trait with a default on-device model and a deterministic,
//! network-free fallback for tests and air-gapped reproducibility.

#[cfg(feature = "ollama")]
use serde::Deserialize;

/// Failure produced by an [`Embedder`] backend (e.g. a network-backed embedder
/// that cannot reach its model). The in-memory [`HashEmbedder`] never fails.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The embedding backend (network, subprocess, …) returned an error.
    #[error("embedding backend error: {0}")]
    Backend(String),
    /// The backend returned an empty embedding vector.
    #[error("embedding backend returned an empty vector")]
    Empty,
}

/// Turns text into a fixed-dimension embedding vector.
pub trait Embedder {
    /// Embedding dimension produced by [`Embedder::embed`].
    fn dimension(&self) -> usize;

    /// Embed `text` into a vector of length [`Embedder::dimension`].
    ///
    /// # Errors
    /// Returns [`EmbedError`] if the backend cannot produce an embedding.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>;
}

/// Deterministic, network-free embedder (token-hashing into L2-normalized
/// buckets). Not semantically strong — its purpose is reproducible tests and
/// offline behavior, exactly like the `fake_embed` used in the repo's
/// `agent_memory` examples. Swap in a real model (e.g. `fastembed`,
/// all-MiniLM-L6-v2, 384-dim) for production recall quality.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dimension: usize,
}

impl HashEmbedder {
    /// Create a [`HashEmbedder`] producing vectors of `dimension` length.
    /// Use `384` to match the SDK's `DEFAULT_DIMENSION`.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

impl Embedder for HashEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut vector = vec![0.0_f32; self.dimension];
        if self.dimension == 0 {
            return Ok(vector);
        }
        let modulus = self.dimension as u64;
        for token in text.split_whitespace() {
            let bucket = usize::try_from(crate::id::stable_id(token) % modulus).unwrap_or(0);
            vector[bucket] += 1.0;
        }
        velesdb_core::simd_native::normalize_inplace_native(&mut vector);
        Ok(vector)
    }
}

/// Forward [`Embedder`] through a box, enabling a non-generic
/// `MemoryService<Box<dyn Embedder + Send + Sync>>` for the MCP server.
impl<T: Embedder + ?Sized> Embedder for Box<T> {
    fn dimension(&self) -> usize {
        (**self).dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        (**self).embed(text)
    }
}

// --- Optional real-recall backend: a local Ollama embeddings endpoint --------
//
// Enabled with `--features ollama`. The default build omits this backend (and
// its HTTP dependency) so the shipped binary stays tiny, zero-dependency, and
// fully offline. This backend keeps the binary small too: it calls a model the
// user already runs locally, so the memory still never leaves the machine.

/// Default Ollama base URL.
#[cfg(feature = "ollama")]
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Default Ollama embedding model (384-dim; `ollama pull all-minilm`).
#[cfg(feature = "ollama")]
pub const DEFAULT_OLLAMA_MODEL: &str = "all-minilm";

/// Embeds text through a local Ollama `/api/embeddings` endpoint — real
/// semantic recall while the model stays on the user's own machine.
#[cfg(feature = "ollama")]
#[derive(Debug, Clone)]
pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    dimension: usize,
}

#[cfg(feature = "ollama")]
impl OllamaEmbedder {
    /// Connect to Ollama at `base_url` using `model`, probing the embedding
    /// dimension once so it adapts to whatever model is configured.
    ///
    /// # Errors
    /// Returns [`EmbedError`] if Ollama is unreachable or the model does not
    /// produce embeddings.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Result<Self, EmbedError> {
        let base_url = base_url.into();
        let model = model.into();
        let dimension = request_embedding(&base_url, &model, "dimension probe")?.len();
        if dimension == 0 {
            return Err(EmbedError::Empty);
        }
        Ok(Self {
            base_url,
            model,
            dimension,
        })
    }
}

#[cfg(feature = "ollama")]
impl Embedder for OllamaEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        request_embedding(&self.base_url, &self.model, text)
    }
}

/// Build the JSON request body for the embeddings endpoint.
#[cfg(feature = "ollama")]
fn build_request_body(model: &str, text: &str) -> String {
    serde_json::json!({ "model": model, "prompt": text }).to_string()
}

/// Ollama `/api/embeddings` response shape.
#[cfg(feature = "ollama")]
#[derive(Deserialize)]
struct EmbeddingResponse {
    embedding: Vec<f32>,
}

/// Parse an embeddings response body into a vector.
#[cfg(feature = "ollama")]
fn parse_embedding_response(body: &str) -> Result<Vec<f32>, EmbedError> {
    let parsed: EmbeddingResponse = serde_json::from_str(body)
        .map_err(|err| EmbedError::Backend(format!("invalid embeddings response: {err}")))?;
    if parsed.embedding.is_empty() {
        return Err(EmbedError::Empty);
    }
    Ok(parsed.embedding)
}

/// Perform one blocking embeddings request against a local Ollama.
#[cfg(feature = "ollama")]
fn request_embedding(base_url: &str, model: &str, text: &str) -> Result<Vec<f32>, EmbedError> {
    let url = format!("{base_url}/api/embeddings");
    let body = build_request_body(model, text);
    let response = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|err| EmbedError::Backend(format!("ollama request failed: {err}")))?;
    let payload = response
        .into_string()
        .map_err(|err| EmbedError::Backend(format!("reading ollama response failed: {err}")))?;
    parse_embedding_response(&payload)
}

#[cfg(all(test, feature = "ollama"))]
mod ollama_tests {
    use super::*;

    #[test]
    fn request_body_carries_model_and_prompt() {
        let body = build_request_body("all-minilm", "hello world");
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(json["model"], "all-minilm");
        assert_eq!(json["prompt"], "hello world");
    }

    #[test]
    fn parses_a_well_formed_embedding() {
        let vector = parse_embedding_response(r#"{"embedding":[0.1,0.2,0.3]}"#).expect("parse");
        assert_eq!(vector.len(), 3);
        assert!((vector[0] - 0.1_f32).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_an_empty_embedding() {
        let parsed = parse_embedding_response(r#"{"embedding":[]}"#);
        assert!(matches!(parsed, Err(EmbedError::Empty)));
    }

    #[test]
    fn rejects_a_malformed_response() {
        let parsed = parse_embedding_response(r#"{"oops":true}"#);
        assert!(matches!(parsed, Err(EmbedError::Backend(_))));
    }

    #[test]
    #[ignore = "requires a local Ollama with an embedding model (ollama pull all-minilm)"]
    fn embeds_through_a_running_ollama() {
        let embedder = OllamaEmbedder::new(DEFAULT_OLLAMA_URL, DEFAULT_OLLAMA_MODEL)
            .expect("connect to ollama");
        let vector = embedder
            .embed("parking_lot avoids lock poisoning")
            .expect("embed");
        assert_eq!(vector.len(), embedder.dimension());
        assert!(vector
            .iter()
            .any(|&component| component.abs() > f32::EPSILON));
    }
}
