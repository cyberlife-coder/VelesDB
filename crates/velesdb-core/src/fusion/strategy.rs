//! Fusion strategies for combining multi-query search results.

#![allow(clippy::unnecessary_wraps)]

use std::collections::HashMap;

/// Error type for fusion operations.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FusionError {
    /// Weights do not sum to 1.0 (within tolerance).
    InvalidWeightSum {
        /// The actual sum of weights.
        sum: f32,
    },
    /// Negative weight provided.
    NegativeWeight {
        /// The negative weight value.
        weight: f32,
    },
    /// Weight slice length does not match the number of result branches.
    WeightCountMismatch {
        /// Number of weights provided.
        weights: usize,
        /// Number of branches passed to fuse.
        branches: usize,
    },
}

impl std::fmt::Display for FusionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWeightSum { sum } => {
                write!(f, "Weights must sum to 1.0, got {sum:.4}")
            }
            Self::NegativeWeight { weight } => {
                write!(f, "Weights must be non-negative, got {weight:.4}")
            }
            Self::WeightCountMismatch { weights, branches } => write!(
                f,
                "WeightedRRF requires one weight per branch: {weights} weights for {branches} branches",
            ),
        }
    }
}

impl std::error::Error for FusionError {}

/// Strategy for fusing results from multiple vector searches.
///
/// Each strategy combines results differently, optimizing for various use cases:
/// - `Average`: Good for general-purpose fusion
/// - `Maximum`: Emphasizes documents that score very high in any query
/// - `RRF`: Position-based fusion, robust to score scale differences
/// - `Weighted`: Custom combination with explicit control over factors
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum FusionStrategy {
    /// Average score across all queries where the document appears.
    ///
    /// Score = mean(scores for this document across queries)
    Average,

    /// Maximum score across all queries.
    ///
    /// Score = max(scores for this document across queries)
    Maximum,

    /// Reciprocal Rank Fusion.
    ///
    /// Score = Σ 1/(k + `rank_i`) for each query where document appears.
    /// Standard k=60 provides good balance between emphasizing top ranks
    /// while still considering lower-ranked results.
    RRF {
        /// Ranking constant (default: 60).
        k: u32,
    },

    /// Weighted combination of average, maximum, and hit ratio.
    ///
    /// Score = `avg_weight` × `avg_score` + `max_weight` × `max_score` + `hit_weight` × `hit_ratio`
    /// where `hit_ratio` = (number of queries containing doc) / (total queries)
    Weighted {
        /// Weight for average score component.
        avg_weight: f32,
        /// Weight for maximum score component.
        max_weight: f32,
        /// Weight for hit ratio component.
        hit_weight: f32,
    },

    /// Relative Score Fusion for dense + sparse hybrid search.
    ///
    /// Each branch is min-max normalized independently, then combined via
    /// weighted sum: `final = dense_weight * norm_dense + sparse_weight * norm_sparse`.
    /// Docs appearing in only one branch get 0 for the missing branch.
    RelativeScore {
        /// Weight for the dense (vector) branch.
        dense_weight: f32,
        /// Weight for the sparse branch.
        sparse_weight: f32,
    },

    /// Weighted Reciprocal Rank Fusion with 0-based ranks.
    ///
    /// Score for document `d` = Σᵢ `weights[i] / (rank_i(d) + k)` where
    /// `rank_i` is the 0-based position of `d` in branch `i`, and `k` is a
    /// smoothing constant (default 60.0).  Documents absent from a branch
    /// contribute nothing from that branch.
    ///
    /// Unlike [`FusionStrategy::RRF`] (which is unweighted and uses 1-based
    /// ranks), this variant gives explicit per-branch control and is the
    /// correct strategy for hybrid dense + text search where branches carry
    /// different retrieval precision characteristics.
    WeightedRRF {
        /// Per-branch non-negative weights; must equal the number of branches
        /// passed to [`FusionStrategy::fuse`].
        weights: Vec<f32>,
        /// Smoothing constant (default 60.0). Higher values dampen the
        /// advantage of the top rank.
        k: f32,
    },
}

impl FusionStrategy {
    /// Creates an RRF strategy with the standard k=60 parameter.
    #[must_use]
    pub fn rrf_default() -> Self {
        Self::RRF { k: 60 }
    }

    /// Creates a `WeightedRRF` strategy with validation.
    ///
    /// # Errors
    ///
    /// Returns an error if any weight is negative or `k` ≤ 0.
    pub fn weighted_rrf(weights: Vec<f32>, k: f32) -> Result<Self, FusionError> {
        validate_non_negative(&weights)?;
        if k <= 0.0 {
            return Err(FusionError::NegativeWeight { weight: k });
        }
        Ok(Self::WeightedRRF { weights, k })
    }

    /// Creates a `RelativeScore` strategy with validation.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Weights do not sum to 1.0 (within 0.001 tolerance)
    /// - Any weight is negative
    pub fn relative_score(dense_weight: f32, sparse_weight: f32) -> Result<Self, FusionError> {
        validate_non_negative(&[dense_weight, sparse_weight])?;
        validate_weight_sum(dense_weight + sparse_weight)?;
        Ok(Self::RelativeScore {
            dense_weight,
            sparse_weight,
        })
    }

    /// Creates a Weighted strategy with validation.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Weights do not sum to 1.0 (within 0.001 tolerance)
    /// - Any weight is negative
    pub fn weighted(
        avg_weight: f32,
        max_weight: f32,
        hit_weight: f32,
    ) -> Result<Self, FusionError> {
        validate_non_negative(&[avg_weight, max_weight, hit_weight])?;
        validate_weight_sum(avg_weight + max_weight + hit_weight)?;

        Ok(Self::Weighted {
            avg_weight,
            max_weight,
            hit_weight,
        })
    }

    /// Fuses results from multiple queries into a single ranked list.
    ///
    /// # Arguments
    ///
    /// * `results` - Vec of search results, one per query. Each inner Vec
    ///   contains `(document_id, score)` tuples, assumed sorted by score descending.
    ///
    /// # Returns
    ///
    /// A single Vec of `(document_id, fused_score)` sorted by score descending.
    ///
    /// # Errors
    ///
    /// Currently infallible, but returns Result for future extensibility.
    pub fn fuse(&self, results: Vec<Vec<(u64, f32)>>) -> Result<Vec<(u64, f32)>, FusionError> {
        if results.is_empty() {
            return Ok(Vec::new());
        }

        // Filter out empty query results for counting
        let non_empty_count = results.iter().filter(|r| !r.is_empty()).count();
        if non_empty_count == 0 {
            return Ok(Vec::new());
        }

        let total_queries = results.len();

        match self {
            Self::Average => Self::fuse_average(results),
            Self::Maximum => Self::fuse_maximum(results),
            Self::RRF { k } => Self::fuse_rrf(results, *k),
            Self::Weighted {
                avg_weight,
                max_weight,
                hit_weight,
            } => Self::fuse_weighted(
                results,
                *avg_weight,
                *max_weight,
                *hit_weight,
                total_queries,
            ),
            Self::RelativeScore {
                dense_weight,
                sparse_weight,
            } => Self::fuse_relative_score(&results, *dense_weight, *sparse_weight),
            Self::WeightedRRF { weights, k } => Self::fuse_weighted_rrf(results, weights, *k),
        }
    }

    /// Collects per-document best scores across queries (deduplicates within each query).
    ///
    /// Returns a map from document ID to the list of its best scores (one per query
    /// where it appeared).
    fn collect_doc_scores(results: Vec<Vec<(u64, f32)>>) -> HashMap<u64, Vec<f32>> {
        let mut doc_scores: HashMap<u64, Vec<f32>> = HashMap::new();

        for query_results in results {
            let mut query_best: HashMap<u64, f32> = HashMap::new();
            for (id, score) in query_results {
                query_best
                    .entry(id)
                    .and_modify(|s| *s = s.max(score))
                    .or_insert(score);
            }

            for (id, score) in query_best {
                doc_scores.entry(id).or_default().push(score);
            }
        }

        doc_scores
    }

    /// Sorts a fused result set by score descending.
    fn sort_descending(fused: &mut [(u64, f32)]) {
        fused.sort_unstable_by(|a, b| b.1.total_cmp(&a.1));
    }

    /// Average fusion: mean of scores for each document.
    #[allow(clippy::cast_precision_loss)]
    // Reason: scores.len() is the number of queries a document appeared in;
    // this is a small count that fits exactly in f32.
    fn fuse_average(results: Vec<Vec<(u64, f32)>>) -> Result<Vec<(u64, f32)>, FusionError> {
        let mut fused: Vec<(u64, f32)> = Self::collect_doc_scores(results)
            .into_iter()
            .map(|(id, scores)| {
                let avg = scores.iter().sum::<f32>() / scores.len() as f32;
                (id, avg)
            })
            .collect();

        Self::sort_descending(&mut fused);
        Ok(fused)
    }

    /// Maximum fusion: best score for each document.
    fn fuse_maximum(results: Vec<Vec<(u64, f32)>>) -> Result<Vec<(u64, f32)>, FusionError> {
        let mut doc_max: HashMap<u64, f32> = HashMap::new();

        for query_results in results {
            for (id, score) in query_results {
                doc_max
                    .entry(id)
                    .and_modify(|s| *s = s.max(score))
                    .or_insert(score);
            }
        }

        let mut fused: Vec<(u64, f32)> = doc_max.into_iter().collect();
        Self::sort_descending(&mut fused);
        Ok(fused)
    }

    /// RRF fusion: reciprocal rank fusion.
    #[allow(clippy::cast_precision_loss)]
    // Reason: k (u32, typically 60) and rank+1 (small loop index) both fit
    // exactly in f32 (exact up to 2^24).
    fn fuse_rrf(results: Vec<Vec<(u64, f32)>>, k: u32) -> Result<Vec<(u64, f32)>, FusionError> {
        let mut doc_rrf: HashMap<u64, f32> = HashMap::new();
        // Reason: k is the RRF constant (default 60, max u32); u32 → f32 is
        // exact for values <= 16_777_216, so no precision loss in practice.
        let k_f32 = k as f32;

        for query_results in results {
            // Deduplicate and get rank order
            let mut seen: HashMap<u64, usize> = HashMap::new();
            for (rank, (id, _score)) in query_results.into_iter().enumerate() {
                // Only count first occurrence (best rank) for each doc in this query
                seen.entry(id).or_insert(rank);
            }

            for (id, rank) in seen {
                let rrf_score = 1.0 / (k_f32 + (rank + 1) as f32);
                *doc_rrf.entry(id).or_insert(0.0) += rrf_score;
            }
        }

        let mut fused: Vec<(u64, f32)> = doc_rrf.into_iter().collect();
        Self::sort_descending(&mut fused);
        Ok(fused)
    }

    /// Weighted fusion: combination of avg, max, and hit ratio.
    ///
    /// # Errors
    ///
    /// Returns an error if the weights are negative or do not sum to 1.0
    /// (within 0.001 tolerance). Validation runs here as well as in the
    /// `weighted()` constructor so that direct enum-literal construction
    /// (e.g. from server/CLI request fields) cannot bypass it.
    #[allow(clippy::cast_precision_loss)]
    // Reason: total_queries and scores.len() are small counts (number of
    // queries/hits per document); both fit exactly in f32.
    fn fuse_weighted(
        results: Vec<Vec<(u64, f32)>>,
        avg_weight: f32,
        max_weight: f32,
        hit_weight: f32,
        total_queries: usize,
    ) -> Result<Vec<(u64, f32)>, FusionError> {
        validate_non_negative(&[avg_weight, max_weight, hit_weight])?;
        validate_weight_sum(avg_weight + max_weight + hit_weight)?;

        let total_q = total_queries as f32;

        let mut fused: Vec<(u64, f32)> = Self::collect_doc_scores(results)
            .into_iter()
            .map(|(id, scores)| {
                let avg = scores.iter().sum::<f32>() / scores.len() as f32;
                let max = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                let hit_ratio = scores.len() as f32 / total_q;

                let combined = avg_weight * avg + max_weight * max + hit_weight * hit_ratio;
                (id, combined)
            })
            .collect();

        Self::sort_descending(&mut fused);
        Ok(fused)
    }

    /// Relative Score Fusion: per-branch min-max normalization + weighted sum.
    ///
    /// Expects exactly two branches in `results`: index 0 = dense, index 1 = sparse.
    /// If more branches are provided, only the first two are used; the extras
    /// are silently discarded. A warning is emitted so callers can detect the
    /// accidental multi-branch case during development.
    ///
    /// # Errors
    ///
    /// Returns an error if the weights are negative or do not sum to 1.0
    /// (within 0.001 tolerance). Validation runs here as well as in the
    /// `relative_score()` constructor so that direct enum-literal construction
    /// (e.g. from server/CLI request fields) cannot bypass it.
    fn fuse_relative_score(
        results: &[Vec<(u64, f32)>],
        dense_weight: f32,
        sparse_weight: f32,
    ) -> Result<Vec<(u64, f32)>, FusionError> {
        validate_non_negative(&[dense_weight, sparse_weight])?;
        validate_weight_sum(dense_weight + sparse_weight)?;

        if results.len() > 2 {
            tracing::warn!(
                branch_count = results.len(),
                "RelativeScore fusion received {} branches but only supports 2 (dense + sparse). \
                 Branches beyond index 1 are ignored.",
                results.len(),
            );
        }

        let dense = results.first().map_or(&[][..], |v| v.as_slice());
        let sparse = results.get(1).map_or(&[][..], |v| v.as_slice());

        let norm_dense = min_max_normalize(dense);
        let norm_sparse = min_max_normalize(sparse);

        // Collect all doc IDs — capacity upper-bounds total unique docs.
        let mut all_ids: HashMap<u64, f32> =
            HashMap::with_capacity(norm_dense.len() + norm_sparse.len());
        for (&id, &nd) in &norm_dense {
            let ns = norm_sparse.get(&id).copied().unwrap_or(0.0);
            all_ids.insert(id, dense_weight * nd + sparse_weight * ns);
        }
        // For sparse-only IDs (not in norm_dense), dense contribution is 0.
        for (&id, &ns) in &norm_sparse {
            all_ids.entry(id).or_insert(sparse_weight * ns);
        }

        let mut fused: Vec<(u64, f32)> = all_ids.into_iter().collect();
        Self::sort_descending(&mut fused);
        Ok(fused)
    }

    /// Weighted 0-based RRF: Σᵢ `weight_i / (rank_i + k)`.
    ///
    /// Rank is 0-based (top result has rank 0). Duplicate document IDs within a
    /// branch are deduplicated — only the best (lowest) rank is used.
    ///
    /// # Errors
    ///
    /// Returns [`FusionError::WeightCountMismatch`] if `weights.len()` ≠
    /// `branches.len()`, or [`FusionError::NegativeWeight`] if any weight is
    /// negative or `k` ≤ 0. Validation runs here as well as in the
    /// `weighted_rrf()` constructor so that direct enum-literal construction
    /// cannot bypass it (same rationale as `fuse_weighted`) — `k = 0` with a
    /// rank-0 hit would otherwise produce an infinite score.
    #[allow(clippy::cast_precision_loss)]
    // Reason: rank and k are small positive values; f32 is sufficient.
    fn fuse_weighted_rrf(
        branches: Vec<Vec<(u64, f32)>>,
        weights: &[f32],
        k: f32,
    ) -> Result<Vec<(u64, f32)>, FusionError> {
        validate_non_negative(weights)?;
        if k <= 0.0 {
            return Err(FusionError::NegativeWeight { weight: k });
        }
        if weights.len() != branches.len() {
            return Err(FusionError::WeightCountMismatch {
                weights: weights.len(),
                branches: branches.len(),
            });
        }

        let mut doc_scores: HashMap<u64, f32> = HashMap::new();

        for (branch, &weight) in branches.into_iter().zip(weights.iter()) {
            // Deduplicate within branch: keep only the best (first) rank.
            let mut best_rank: HashMap<u64, usize> = HashMap::new();
            for (rank, (id, _)) in branch.into_iter().enumerate() {
                best_rank.entry(id).or_insert(rank);
            }
            for (id, rank) in best_rank {
                let contribution = weight / (rank as f32 + k);
                *doc_scores.entry(id).or_insert(0.0) += contribution;
            }
        }

        let mut fused: Vec<(u64, f32)> = doc_scores.into_iter().collect();
        Self::sort_descending(&mut fused);
        Ok(fused)
    }
}

impl Default for FusionStrategy {
    fn default() -> Self {
        Self::RRF { k: 60 }
    }
}

// ---------------------------------------------------------------------------
// Shared validation helpers (extracted from `relative_score` / `weighted`)
// ---------------------------------------------------------------------------

/// Validates that no weight in the slice is negative.
fn validate_non_negative(weights: &[f32]) -> Result<(), FusionError> {
    for &w in weights {
        if w < 0.0 {
            return Err(FusionError::NegativeWeight { weight: w });
        }
    }
    Ok(())
}

/// Validates that a weight sum is 1.0 (within 0.001 tolerance).
fn validate_weight_sum(sum: f32) -> Result<(), FusionError> {
    if (sum - 1.0).abs() > 0.001 {
        return Err(FusionError::InvalidWeightSum { sum });
    }
    Ok(())
}

/// Min-max normalize a branch of `(id, score)` pairs.
///
/// If the score range is smaller than `f32::EPSILON`, all items receive 0.5.
fn min_max_normalize(branch: &[(u64, f32)]) -> HashMap<u64, f32> {
    if branch.is_empty() {
        return HashMap::new();
    }
    // Single pass to find both min and max.
    let (min, max) = branch
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &(_, s)| {
            (lo.min(s), hi.max(s))
        });
    let range = max - min;
    let mut out = HashMap::with_capacity(branch.len());
    for &(id, s) in branch {
        let norm = if range < f32::EPSILON {
            0.5
        } else {
            (s - min) / range
        };
        out.insert(id, norm);
    }
    out
}
