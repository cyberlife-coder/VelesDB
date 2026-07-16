//! Deterministic lexical relevance of a fragment to the request query — and
//! [`DeterministicReranker`], the first [`Reranker`] implementation the crate
//! ships (the trait was previously a bring-your-own plug-point only).
//!
//! No model, no randomness: the score is the fraction of the query's distinct
//! lowercase alphanumeric terms that also appear in the fragment, in `[0, 1]`.
//! Coarse on purpose — it only orders same-priority fragments during packing
//! and documents each decision; it never invents or drops content.

use std::collections::BTreeSet;

use crate::model::Recollection;
use crate::rerank::{RerankError, Reranker};

/// The distinct lowercase alphanumeric terms of `text`. Build the query's
/// set once per compile and score every fragment against it — re-tokenizing
/// the query per fragment would be pure loop-invariant waste.
pub(crate) fn terms(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Fraction of the query's distinct terms found in `content`, in `[0, 1]`.
/// An empty query scores every fragment `0.0`.
#[allow(clippy::cast_precision_loss)] // term counts are far below 2^23
pub(crate) fn lexical_relevance(query_terms: &BTreeSet<String>, content: &str) -> f32 {
    if query_terms.is_empty() {
        return 0.0;
    }
    let content_terms = terms(content);
    let overlap = query_terms.intersection(&content_terms).count();
    overlap as f32 / query_terms.len() as f32
}

/// The first shipped [`Reranker`]: re-orders a fused candidate pool by
/// deterministic lexical overlap with the query, original (fused) order as
/// the tie-break. Never invents or drops ids, never calls a model — safe to
/// wire into
/// [`recall_fused_reranked`](crate::service::MemoryService::recall_fused_reranked)
/// where a cross-encoder would be overkill or non-reproducible.
///
/// Like every reranker in this crate it is **opt-in**: lexical overlap can
/// demote a semantically relevant but differently-worded fact, so it suits
/// keyword-anchored agent queries better than open conversational ones.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicReranker;

impl Reranker for DeterministicReranker {
    fn rerank(
        &self,
        query: &str,
        candidates: Vec<Recollection>,
    ) -> Result<Vec<Recollection>, RerankError> {
        let query_terms = terms(query);
        let mut indexed: Vec<(usize, f32, Recollection)> = candidates
            .into_iter()
            .enumerate()
            .map(|(position, candidate)| {
                let score = lexical_relevance(&query_terms, &candidate.content);
                (position, score, candidate)
            })
            .collect();
        indexed.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(indexed
            .into_iter()
            .map(|(_, _, candidate)| candidate)
            .collect())
    }
}

#[cfg(test)]
#[path = "relevance_tests.rs"]
mod tests;
