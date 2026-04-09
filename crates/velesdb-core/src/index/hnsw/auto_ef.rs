//! Auto-tuned `ef_search` range based on collection statistics.
//!
//! Computes optimal min/max `ef_search` values for adaptive search
//! by considering the collection size, vector dimension, and desired k.

/// Computes optimal `ef_search` range from collection statistics.
///
/// Returns `(min_ef, max_ef)` for use with [`SearchQuality::Adaptive`].
///
/// # Strategy
///
/// - **Base ef** scales logarithmically with collection size: larger
///   collections need broader exploration to maintain recall.
/// - **Dimension factor**: high-dimensional spaces (>512) have sparser
///   neighborhoods, requiring more candidates for the same recall.
/// - **`min_ef`** is clamped to at least `k` (never fewer candidates than
///   requested results).
/// - **`max_ef`** is set to `4 * min_ef`, giving the adaptive second phase
///   ample headroom for hard queries.
///
/// [`SearchQuality::Adaptive`]: super::params::SearchQuality::Adaptive
#[must_use]
pub(crate) fn auto_ef_range(count: usize, dimension: usize, k: usize) -> (usize, usize) {
    let base = collection_size_base_ef(count, k);
    let dim_factor = dimension_factor(dimension);

    // Reason: base <= k*12 (at most ~120K for k=10K), dim_factor <= 1.5.
    // Product is always small and positive, so f64 precision loss and
    // truncation to usize are both harmless.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let min_ef = ((base as f64 * dim_factor) as usize).max(k);
    let max_ef = min_ef.saturating_mul(4);

    (min_ef, max_ef)
}

/// Base ef scaling by collection size.
///
/// Larger collections have more nodes to explore; the multiplier
/// grows in discrete tiers to avoid over-exploring small datasets.
#[inline]
const fn collection_size_base_ef(count: usize, k: usize) -> usize {
    match count {
        0..=1_000 => k * 2,
        1_001..=10_000 => k * 4,
        10_001..=100_000 => k * 8,
        _ => k * 12,
    }
}

/// Dimension adjustment factor for high-dimensional spaces.
///
/// Vectors with >512 dimensions live in sparser neighborhoods,
/// so the search frontier needs to be wider to achieve the same recall.
#[inline]
fn dimension_factor(dimension: usize) -> f64 {
    if dimension > 512 {
        1.5
    } else {
        1.0
    }
}
