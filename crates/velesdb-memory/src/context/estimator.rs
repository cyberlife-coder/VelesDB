//! Pluggable token estimation, with a deterministic char-ratio default.
//!
//! Same shape as [`crate::embedder`]: a small trait, one dependency-free
//! default, and a boxed alias for non-generic holders. Estimates here are
//! **local approximations** — distinct from a provider's exact tokenizer
//! count, from billed tokens, and from cache-read tokens. Budget packing
//! treats them as an over-approximation on purpose: refusing a borderline
//! fragment is recoverable (it becomes a retrieval handle), overflowing the
//! window is not.

/// Turns text into an estimated token count.
pub trait TokenEstimator {
    /// Estimated number of tokens `text` occupies in a model prompt.
    fn estimate(&self, text: &str) -> u64;

    /// Rough bytes-per-token ratio of this estimator, used only as a *hint*
    /// to size chunk pieces near the budget (every piece is still measured
    /// by [`Self::estimate`] during packing, so a wrong hint costs
    /// granularity, never correctness). The default matches the char-ratio
    /// estimator; a model-exact tokenizer for dense scripts (CJK) should
    /// lower it.
    fn bytes_per_token_hint(&self) -> u64 {
        2
    }
}

/// A boxed, object-safe estimator, mirroring [`crate::embedder::DynEmbedder`].
pub type DynTokenEstimator = Box<dyn TokenEstimator + Send + Sync>;

/// Forward [`TokenEstimator`] through a box so a non-generic compiler can
/// hold [`DynTokenEstimator`].
impl<T: TokenEstimator + ?Sized> TokenEstimator for Box<T> {
    fn estimate(&self, text: &str) -> u64 {
        (**self).estimate(text)
    }

    fn bytes_per_token_hint(&self) -> u64 {
        (**self).bytes_per_token_hint()
    }
}

/// Deterministic char-ratio estimator: `ceil(chars × 2 ÷ 5)` (= 2.5 chars per
/// token), the ratio the `LoCoMo` harness calibrated against its Latin-script
/// corpus (which measured ~2.9 chars/token) so the estimate deliberately
/// **over-counts** by ~15 %. Known bias: heavily non-Latin text (CJK, emoji)
/// tokenizes to *more* than one token per char, which this ratio
/// under-counts — inject a model-specific [`TokenEstimator`] when compiling
/// such corpora against a tight budget.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicEstimator;

impl TokenEstimator for HeuristicEstimator {
    fn estimate(&self, text: &str) -> u64 {
        let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
        chars.saturating_mul(2).div_ceil(5)
    }
}

#[cfg(test)]
#[path = "estimator_tests.rs"]
mod tests;
