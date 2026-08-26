//! Distance metrics for vector similarity calculations.
//!
//! # Performance
//!
//! All distance calculations use direct SIMD dispatch via `simd_native` module,
//! eliminating intermediate dispatch overhead for maximum performance:
//! - **Cosine**: Direct AVX-512/AVX2/NEON intrinsics
//! - **Euclidean**: Direct native intrinsics with 4-acc unrolling
//! - **Dot Product**: Direct FMA-optimized intrinsics
//! - **Hamming (binary)**: `DistanceMetric::calculate` uses the f32 variant
//!   (0.5 threshold per component); the POPCNT-on-packed-u64 fast path
//!   (~48x faster) is a separate API consumed by the `RaBitQ` pipeline
//! - **Jaccard**: Set similarity with SIMD acceleration

use crate::simd_native;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Canonical names of every [`DistanceMetric`] variant, in declaration order.
///
/// Single source of truth for the metric name set exported to downstream
/// crates and bindings (Python `velesdb.DISTANCE_METRICS`, the integrations
/// security guard). Each entry is the variant's
/// [`canonical_name`](DistanceMetric::canonical_name); a unit test asserts the
/// slice stays exhaustive so adding a variant without updating it fails CI.
pub const DISTANCE_METRIC_NAMES: &[&str] = &["cosine", "euclidean", "dot", "hamming", "jaccard"];

/// Canonical serde `type` tags of every [`Condition`](crate::filter::Condition)
/// variant, in declaration order.
///
/// Single source of truth for the filter condition-type vocabulary exported to
/// downstream crates and bindings (the integrations' filter conversion and the
/// MIT-side security guard) so they never re-derive the tag spelling as literals
/// and drift when a variant is added. A unit test in `crate::filter` asserts the
/// slice round-trips against the actual serde representation so adding a variant
/// without updating it fails CI.
pub const CONDITION_TYPE_NAMES: &[&str] = &[
    "eq",
    "neq",
    "gt",
    "gte",
    "lt",
    "lte",
    "in",
    "contains",
    "is_null",
    "is_not_null",
    "and",
    "or",
    "not",
    "like",
    "ilike",
    "array_contains",
    "array_contains_any",
    "array_contains_all",
    "geo_distance",
    "geo_bbox",
];

/// Distance metric for vector similarity calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DistanceMetric {
    /// Cosine similarity (1 - `cosine_distance`).
    /// Best for normalized vectors, commonly used with text embeddings.
    Cosine,

    /// Euclidean distance (L2 norm).
    /// Best for spatial data and when magnitude matters.
    Euclidean,

    /// Dot product (inner product).
    /// Best for maximum inner product search (MIPS).
    DotProduct,

    /// Hamming distance for binary vectors.
    /// Counts the number of positions where bits differ.
    /// Best for binary embeddings and locality-sensitive hashing.
    Hamming,

    /// Jaccard similarity for set-like vectors.
    /// Measures intersection over union of non-zero elements.
    /// Best for sparse vectors, tags, and set membership.
    Jaccard,
}

impl DistanceMetric {
    /// Returns the canonical metric name used by user-facing APIs.
    #[must_use]
    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Cosine => "cosine",
            Self::Euclidean => "euclidean",
            Self::DotProduct => "dot",
            Self::Hamming => "hamming",
            Self::Jaccard => "jaccard",
        }
    }

    /// Parses a metric name/alias into a [`DistanceMetric`].
    ///
    /// Supported aliases:
    /// - cosine
    /// - euclidean, l2
    /// - dot, dotproduct, inner, ip
    /// - hamming
    /// - jaccard
    #[must_use]
    pub fn parse_alias(value: &str) -> Option<Self> {
        match value.trim().to_lowercase().as_str() {
            "cosine" => Some(Self::Cosine),
            "euclidean" | "l2" => Some(Self::Euclidean),
            "dot" | "dotproduct" | "inner" | "ip" => Some(Self::DotProduct),
            "hamming" => Some(Self::Hamming),
            "jaccard" => Some(Self::Jaccard),
            _ => None,
        }
    }

    /// Calculates the distance between two vectors using the specified metric.
    ///
    /// # Arguments
    ///
    /// * `a` - First vector
    /// * `b` - Second vector
    ///
    /// # Returns
    ///
    /// Distance value (lower is more similar for Euclidean, higher for Cosine/DotProduct).
    ///
    /// # Panics
    ///
    /// Panics if vectors have different dimensions.
    ///
    /// # Performance
    ///
    /// Uses SIMD-optimized implementations. Typical latencies for 768d vectors:
    /// - Cosine: ~32ns
    /// - Euclidean: ~20ns
    /// - Dot Product: ~18ns
    #[must_use]
    #[inline]
    pub fn calculate(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            Self::Cosine => simd_native::cosine_similarity_native(a, b),
            Self::Euclidean => simd_native::euclidean_native(a, b),
            Self::DotProduct => simd_native::dot_product_native(a, b),
            Self::Hamming => simd_native::hamming_distance_native(a, b),
            Self::Jaccard => simd_native::jaccard_similarity_native(a, b),
        }
    }

    /// Returns whether higher values indicate more similarity.
    #[must_use]
    pub const fn higher_is_better(&self) -> bool {
        match self {
            Self::Cosine | Self::DotProduct | Self::Jaccard => true,
            Self::Euclidean | Self::Hamming => false,
        }
    }

    /// Returns whether an orthogonal change of basis leaves this metric
    /// unchanged.
    ///
    /// OPQ stores codes in a rotated space and rescoring rotates the query to
    /// match. That is sound only for metrics built from inner products and
    /// norms, which a rotation preserves: cosine, dot product, Euclidean.
    /// Hamming and Jaccard are defined component by component on the original
    /// axes — a rotated "binary" vector is no longer binary — so the same
    /// trick computes a number that has no relation to the metric the user
    /// asked for.
    #[must_use]
    pub const fn is_rotation_invariant(&self) -> bool {
        match self {
            Self::Cosine | Self::Euclidean | Self::DotProduct => true,
            Self::Hamming | Self::Jaccard => false,
        }
    }

    /// Returns this metric's score polarity for fusion
    /// ([`ScoreDirection`](crate::fusion::ScoreDirection)).
    ///
    /// Derived from [`Self::higher_is_better`] so the two can never drift.
    #[must_use]
    pub const fn score_direction(&self) -> crate::fusion::ScoreDirection {
        if self.higher_is_better() {
            crate::fusion::ScoreDirection::HigherIsBetter
        } else {
            crate::fusion::ScoreDirection::LowerIsBetter
        }
    }

    /// The closed range this metric's user-visible scores occupy, or `None`
    /// when the metric is unbounded.
    ///
    /// This is the one definition of the score contract. A caller cannot tell
    /// which path produced its results — `HnswIndex` reaches a score through
    /// brute force, through the exact rerank, or through the graph's
    /// `transform_score`, chosen by corpus size, quality and `k` — so those
    /// paths must not each carry their own table. They derive from this one.
    ///
    /// - **Cosine** is `[-1, 1]`. An anti-correlated pair has a genuinely
    ///   negative similarity; flooring it at zero does not merely lose the
    ///   sign, it ties every anti-correlated match at one value and destroys
    ///   the ordering among them.
    /// - **Jaccard** is `[0, 1]`: `intersection / union` over non-negative
    ///   weights cannot go below zero.
    /// - **Euclidean**, **Hamming** and **`DotProduct`** are unbounded — a
    ///   distance grows with the data, and an inner product is not normalized.
    #[must_use]
    pub const fn score_range(&self) -> Option<(f32, f32)> {
        match self {
            Self::Cosine => Some((-1.0, 1.0)),
            Self::Jaccard => Some((0.0, 1.0)),
            Self::Euclidean | Self::Hamming | Self::DotProduct => None,
        }
    }

    /// Clamps `score` into [`Self::score_range`], leaving unbounded metrics
    /// untouched.
    ///
    /// Absorbs floating-point drift at the boundary without moving a value
    /// that is genuinely inside the range.
    #[must_use]
    pub fn clamp_score(&self, score: f32) -> f32 {
        match self.score_range() {
            Some((low, high)) => score.clamp(low, high),
            None => score,
        }
    }

    /// Sorts search results by distance/similarity according to the metric.
    ///
    /// - **Similarity metrics** (`Cosine`, `DotProduct`, `Jaccard`): sorts descending (higher = better)
    /// - **Distance metrics** (`Euclidean`, `Hamming`): sorts ascending (lower = better)
    ///
    /// Generic over the id type so both external ids (`u64`) and internal
    /// HNSW node ids (`usize`) can be sorted with the same metric semantics.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let mut results = vec![(1, 0.9), (2, 0.7), (3, 0.8)];
    /// DistanceMetric::Cosine.sort_results(&mut results);
    /// assert_eq!(results[0].0, 1); // Highest similarity first
    /// ```
    pub fn sort_results<Id>(&self, results: &mut [(Id, f32)]) {
        if self.higher_is_better() {
            // Similarity metrics: descending order (higher = better)
            results.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
        } else {
            // Distance metrics: ascending order (lower = better)
            results.sort_unstable_by(|a, b| a.1.total_cmp(&b.1));
        }
    }

    /// Sorts scored results by the distance metric semantics.
    ///
    /// Same as [`sort_results`](Self::sort_results) but operates on
    /// [`ScoredResult`](crate::scored_result::ScoredResult) slices.
    pub fn sort_scored_results(&self, results: &mut [crate::scored_result::ScoredResult]) {
        if self.higher_is_better() {
            results.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
        } else {
            results.sort_unstable_by(|a, b| a.score.total_cmp(&b.score));
        }
    }

    /// Top-k twin of [`Self::sort_scored_results`]: O(n + k log k) partial
    /// select + sort instead of a full O(n log n) sort, for callers that
    /// truncate to `k` anyway — the bitmap brute-force scan hands this the
    /// whole allowed set, which can be a large fraction of the collection.
    ///
    /// Gated with the `index` module its helper lives in (its callers are
    /// the persistence-gated HNSW paths).
    #[cfg(feature = "persistence")]
    pub(crate) fn top_k_scored_results(
        self,
        results: &mut Vec<crate::scored_result::ScoredResult>,
        k: usize,
    ) {
        if self.higher_is_better() {
            crate::index::top_k_partial_sort(results, k, |a, b| b.score.total_cmp(&a.score));
        } else {
            crate::index::top_k_partial_sort(results, k, |a, b| a.score.total_cmp(&b.score));
        }
    }
}

impl FromStr for DistanceMetric {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_alias(s)
            .ok_or("Unknown metric. Use: cosine, euclidean, l2, dot, dotproduct, inner, ip, hamming, jaccard")
    }
}

#[cfg(test)]
#[path = "distance_metric_names_tests.rs"]
mod distance_metric_names_tests;
