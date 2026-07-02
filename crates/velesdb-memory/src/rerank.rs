//! Optional second-stage re-scoring of a [`MemoryService::recall_fused`]
//! candidate pool, the layer that lifts a ranking miss (a relevant fact deep
//! in the pool, below the fusion cutoff) into the final `k` — the lever
//! validated on the LoCoMo ceiling diagnostic (multi-hop recall@8 = 50%,
//! recall@64 = 89%: the gold fact is IN the pool, just outranked).
//!
//! Mirroring the [`crate::embedder`]/[`crate::extract`] pattern, the
//! plug-point is dependency-free (bring your own cross-encoder or LLM by
//! implementing [`Reranker`]) and never wired in by default: a reranker
//! measurably helps ranking-bound corpora but can hurt on out-of-distribution
//! conversational queries (the LoCoMo panel's own finding) — opt-in only,
//! never a silent default.
//!
//! [`MemoryService::recall_fused`]: crate::service::MemoryService::recall_fused

use crate::model::Recollection;

/// Failure produced by a [`Reranker`] backend (e.g. a network-backed model
/// that cannot be reached, or output that cannot be mapped back to
/// candidates).
#[derive(Debug, thiserror::Error)]
pub enum RerankError {
    /// The reranking backend (network, subprocess, …) returned an error.
    #[error("rerank backend error: {0}")]
    Backend(String),
}

/// Re-scores or reorders a fused candidate pool against `query`.
///
/// Implement this to plug in a cross-encoder, an LLM judge, or any other
/// second-stage ranker, and pass it to
/// [`MemoryService::recall_fused_reranked`](crate::service::MemoryService::recall_fused_reranked).
/// A well-behaved implementation returns the same candidates (by id), just
/// reordered or re-scored — it should not invent or drop ids.
pub trait Reranker {
    /// Re-score or reorder `candidates` for relevance to `query`.
    ///
    /// # Errors
    /// Returns [`RerankError`] if the backend fails.
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<Recollection>,
    ) -> Result<Vec<Recollection>, RerankError>;
}

/// Forward [`Reranker`] through an [`Arc`], so a shared `Arc<dyn Reranker>`
/// (e.g. one held by the MCP server) satisfies the `R: Reranker` bound on
/// [`crate::MemoryService::recall_fused_reranked`].
impl<T: Reranker + ?Sized> Reranker for std::sync::Arc<T> {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<Recollection>,
    ) -> Result<Vec<Recollection>, RerankError> {
        (**self).rerank(query, candidates)
    }
}

/// A boxed, object-safe reranker. Lets a non-generic caller (e.g. the Node
/// binding, wrapping a JS callback) hold `Box<dyn Reranker + Send + Sync>`
/// without threading a generic parameter through `MemoryService`.
pub type DynReranker = Box<dyn Reranker + Send + Sync>;

impl Reranker for DynReranker {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<Recollection>,
    ) -> Result<Vec<Recollection>, RerankError> {
        (**self).rerank(query, candidates)
    }
}
