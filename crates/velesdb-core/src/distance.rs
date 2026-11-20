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
//!   is a separate API consumed by the `RaBitQ` pipeline
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

/// The aliases of [`METRIC_ALIASES`], comma-separated, as a literal: the
/// infallible [`FromStr`] error is a `&'static str`, so it is built with
/// `concat!`, which takes a literal and not a `const`.
/// `the_listed_aliases_are_exactly_the_parsed_ones` keeps the two in step.
macro_rules! metric_alias_list {
    () => {
        "cosine, euclidean, l2, dot, dotproduct, inner, ip, hamming, jaccard"
    };
}

/// Every name [`DistanceMetric::parse_alias`] accepts, and the metric it
/// stands for: the one alias table, read by the parser, the `VelesQL` DDL
/// validation and every unknown-metric message alike.
const METRIC_ALIASES: [(&str, DistanceMetric); 9] = [
    ("cosine", DistanceMetric::Cosine),
    ("euclidean", DistanceMetric::Euclidean),
    ("l2", DistanceMetric::Euclidean),
    ("dot", DistanceMetric::DotProduct),
    ("dotproduct", DistanceMetric::DotProduct),
    ("inner", DistanceMetric::DotProduct),
    ("ip", DistanceMetric::DotProduct),
    ("hamming", DistanceMetric::Hamming),
    ("jaccard", DistanceMetric::Jaccard),
];

/// The accepted metric aliases as every unknown-metric message lists them.
pub(crate) const METRIC_ALIAS_LIST: &str = metric_alias_list!();

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

    /// Parses a metric name/alias into a [`DistanceMetric`], trimmed and
    /// case-insensitively.
    ///
    /// Supported aliases:
    /// - cosine
    /// - euclidean, l2
    /// - dot, dotproduct, inner, ip
    /// - hamming
    /// - jaccard
    #[must_use]
    pub fn parse_alias(value: &str) -> Option<Self> {
        let value = value.trim().to_lowercase();
        METRIC_ALIASES
            .iter()
            .find(|(name, _)| *name == value)
            .map(|&(_, metric)| metric)
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
    /// Uses SIMD-optimized implementations. Their measured latencies, each
    /// with the run behind it, are in `docs/BENCHMARKS.md`.
    #[must_use]
    #[inline]
    pub fn calculate(&self, a: &[f32], b: &[f32]) -> f32 {
        match self {
            Self::Cosine => simd_native::cosine_similarity_native(a, b),
            Self::Euclidean => simd_native::euclidean_native(a, b),
            Self::DotProduct => simd_native::dot_product_native(a, b),
            Self::Hamming => simd_native::hamming_distance_native(a, b),
            // The kernels return the raw `intersection / union` ratio, which
            // escapes the declared `[0, 1]` on out-of-contract (negative-weight)
            // input; `calculate` is the metric-score layer, so the declared
            // range is enforced here (#2095). The kernel itself stays raw —
            // `simd_property_tests` pins it against an f64 reference.
            Self::Jaccard => simd_native::jaccard_similarity_native(a, b).clamp(0.0, 1.0),
        }
    }

    /// [`calculate`](Self::calculate) when `a` and `b` can be compared: the
    /// same length, not empty. `None` otherwise, where `calculate` would
    /// panic.
    ///
    /// The one guard a score read from a row goes through, in core's
    /// executor and the WASM one alike: a row hydrated without a vector (a
    /// payload-only graph node, a metadata row) carries an empty one, and a
    /// payload's named vector can have any length, so neither may reach a
    /// kernel's length assertion.
    ///
    /// ```
    /// use velesdb_core::DistanceMetric;
    ///
    /// assert_eq!(DistanceMetric::Cosine.checked_calculate(&[], &[1.0, 0.0]), None);
    /// assert!(DistanceMetric::Cosine.checked_calculate(&[1.0, 0.0], &[1.0, 0.0]).is_some());
    /// ```
    #[must_use]
    #[inline]
    pub fn checked_calculate(self, a: &[f32], b: &[f32]) -> Option<f32> {
        (a.len() == b.len() && !a.is_empty()).then(|| self.calculate(a, b))
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
    ///
    /// A NaN score ranks last whatever its sign (see [`rank_scores`]).
    pub fn sort_results<Id>(&self, results: &mut [(Id, f32)]) {
        let higher_is_better = self.higher_is_better();
        results.sort_unstable_by(|a, b| rank_scores(a.1, b.1, higher_is_better));
    }

    /// Sorts scored results by the distance metric semantics.
    ///
    /// Same as [`sort_results`](Self::sort_results) but operates on
    /// [`ScoredResult`](crate::scored_result::ScoredResult) slices.
    pub fn sort_scored_results(&self, results: &mut [crate::scored_result::ScoredResult]) {
        let higher_is_better = self.higher_is_better();
        results.sort_unstable_by(|a, b| rank_scores(a.score, b.score, higher_is_better));
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
        let higher_is_better = self.higher_is_better();
        crate::index::top_k_partial_sort(results, k, |a, b| {
            rank_scores(a.score, b.score, higher_is_better)
        });
    }
}

/// Orders two metric scores best first, with NaN last.
///
/// `higher_is_better` is [`DistanceMetric::higher_is_better`]: descending for
/// similarities, ascending for distances. A NaN (a degenerate vector, or an
/// overflow such as `inf - inf` in a dot product) is never a match, so it
/// ranks after every number in both directions. Bare [`f32::total_cmp`]
/// would place it by its sign bit instead, and that sign is whatever the
/// hardware produced: a runtime `0.0 / 0.0` is a negative NaN on `x86_64`, a
/// positive one on `aarch64`, so the same query ranked a NaN first on one
/// architecture and last on the other. `-0.0` and `0.0` tie, as in
/// [`crate::fusion::sort_fused_results`].
///
/// Public so `velesdb-wasm` ranks with the same comparator as core instead of
/// a copy. Fusion orders NaN differently on purpose, see
/// [`crate::fusion::sort_fused_results`].
#[must_use]
pub fn rank_scores(a: f32, b: f32, higher_is_better: bool) -> std::cmp::Ordering {
    let (first, second) = if higher_is_better { (b, a) } else { (a, b) };
    match (a.is_nan(), b.is_nan()) {
        // Neither is NaN, so `partial_cmp` is total here; it is used rather
        // than `total_cmp` so that `-0.0` and `0.0`, one score, tie.
        (false, false) => first
            .partial_cmp(&second)
            .unwrap_or(std::cmp::Ordering::Equal),
        // `false < true`: the NaN side compares greater, i.e. later.
        (a_nan, b_nan) => a_nan.cmp(&b_nan),
    }
}

/// A higher-is-better score as an integer whose order is [`rank_scores`]'s
/// (`higher_is_better`), the better greater: a NaN least whatever its sign
/// bit, and `-0.0` equal to `0.0` — by the bit pattern of IEEE 754's total
/// order. One integer comparison per pair, for a heap that compares often
/// (`collection/search/ranked.rs`, the sparse search's top-k).
#[must_use]
pub(crate) fn score_order_key(score: f32) -> u32 {
    if score.is_nan() {
        return 0;
    }
    let bits = if score == 0.0 { 0.0_f32 } else { score }.to_bits();
    // Negative: every bit flipped, so a larger magnitude is lower; no number
    // maps to 0, the all-ones pattern flipping to it being a NaN.
    if bits >> 31 == 1 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}

/// A hit's place in a ranking, as one integer: the greatest ranks first. The
/// high half orders the score, the better greater ([`score_order_key`]: a
/// NaN least whatever its sign bit, `-0.0` equal to `0.0`); the low half
/// orders a tie by id, the lower greater — the tie order of a fused ranking
/// (`crate::fusion::sort_fused_results`). The one rule a text, sparse or
/// ranked walk orders its hits by, so a cut at any depth keeps the ids
/// fusion would put first.
#[cfg(feature = "persistence")]
pub(crate) fn hit_rank_key(score: f32, id: u64) -> u128 {
    (u128::from(score_order_key(score)) << 64) | u128::from(u64::MAX - id)
}

impl FromStr for DistanceMetric {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_alias(s).ok_or(concat!("Unknown metric. Use: ", metric_alias_list!()))
    }
}

#[cfg(test)]
#[path = "distance_metric_names_tests.rs"]
mod distance_metric_names_tests;
