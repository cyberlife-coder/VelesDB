//! Pluggable text → vector embedding.
//!
//! The Agent Memory SDK is *bring-your-own-vector*: it never generates
//! embeddings. This crate mirrors the repo's established pattern (the Python
//! SDK's `Embedder` protocol, the tauri-rag demo's `fastembed` backend): an
//! [`Embedder`] trait with a default on-device model and a deterministic,
//! network-free fallback for tests and air-gapped reproducibility.

/// Turns text into a fixed-dimension embedding vector.
pub trait Embedder {
    /// Embedding dimension produced by [`Embedder::embed`].
    fn dimension(&self) -> usize;

    /// Embed `text` into a vector of length [`Embedder::dimension`].
    fn embed(&self, text: &str) -> Vec<f32>;
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

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0_f32; self.dimension];
        if self.dimension == 0 {
            return vector;
        }
        let modulus = self.dimension as u64;
        for token in text.split_whitespace() {
            let bucket = usize::try_from(crate::id::stable_id(token) % modulus).unwrap_or(0);
            vector[bucket] += 1.0;
        }
        velesdb_core::simd_native::normalize_inplace_native(&mut vector);
        vector
    }
}

/// Forward [`Embedder`] through a box, enabling a non-generic
/// `MemoryService<Box<dyn Embedder + Send + Sync>>` for the MCP server.
impl<T: Embedder + ?Sized> Embedder for Box<T> {
    fn dimension(&self) -> usize {
        (**self).dimension()
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        (**self).embed(text)
    }
}
