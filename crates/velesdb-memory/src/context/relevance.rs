//! Deterministic lexical relevance of a fragment to the request query.
//!
//! No model, no randomness: the score is the fraction of the query's distinct
//! lowercase alphanumeric terms that also appear in the fragment, in `[0, 1]`.
//! Coarse on purpose — it only orders same-priority fragments during packing
//! and documents each decision; it never invents or drops content. US-002
//! layers the first shipped [`crate::rerank::Reranker`] implementation on the
//! same normalization.

use std::collections::BTreeSet;

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

#[cfg(test)]
#[path = "relevance_tests.rs"]
mod tests;
