//! Fusion strategies for combining multi-query search results.

#![allow(clippy::cast_precision_loss)]
#![allow(clippy::unnecessary_wraps)]

use std::collections::HashMap;

/// Error type for fusion operations.
#[derive(Debug, Clone, PartialEq)]
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
}

impl FusionStrategy {
    /// Creates an RRF strategy with the standard k=60 parameter.
    #[must_use]
    pub fn rrf_default() -> Self {
        Self::RRF { k: 60 }
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
        // Validate non-negative
        if avg_weight < 0.0 {
            return Err(FusionError::NegativeWeight { weight: avg_weight });
        }
        if max_weight < 0.0 {
            return Err(FusionError::NegativeWeight { weight: max_weight });
        }
        if hit_weight < 0.0 {
            return Err(FusionError::NegativeWeight { weight: hit_weight });
        }

        // Validate sum to 1.0
        let sum = avg_weight + max_weight + hit_weight;
        if (sum - 1.0).abs() > 0.001 {
            return Err(FusionError::InvalidWeightSum { sum });
        }

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
            } => Ok(Self::fuse_weighted(
                results,
                *avg_weight,
                *max_weight,
                *hit_weight,
                total_queries,
            )),
        }
    }

    /// Average fusion: mean of scores for each document.
    fn fuse_average(results: Vec<Vec<(u64, f32)>>) -> Result<Vec<(u64, f32)>, FusionError> {
        let mut doc_scores: HashMap<u64, Vec<f32>> = HashMap::new();

        for query_results in results {
            // Deduplicate within query (take best score for each doc)
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

        let mut fused: Vec<(u64, f32)> = doc_scores
            .into_iter()
            .map(|(id, scores)| {
                // SAFETY: scores.len() is typically < 1000, fits in f32 with full precision
                #[allow(clippy::cast_precision_loss)]
                let avg = scores.iter().sum::<f32>() / scores.len() as f32;
                (id, avg)
            })
            .collect();

        // Sort by score descending
        fused.sort_by(|a, b| b.1.total_cmp(&a.1));

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
        fused.sort_by(|a, b| b.1.total_cmp(&a.1));

        Ok(fused)
    }

    /// RRF fusion: reciprocal rank fusion.
    fn fuse_rrf(results: Vec<Vec<(u64, f32)>>, k: u32) -> Result<Vec<(u64, f32)>, FusionError> {
        let mut doc_rrf: HashMap<u64, f32> = HashMap::new();
        // SAFETY: k is typically 60, fits in f32 with full precision
        #[allow(clippy::cast_precision_loss)]
        let k_f32 = k as f32;

        for query_results in results {
            // Deduplicate and get rank order
            let mut seen: HashMap<u64, usize> = HashMap::new();
            for (rank, (id, _score)) in query_results.into_iter().enumerate() {
                // Only count first occurrence (best rank) for each doc in this query
                seen.entry(id).or_insert(rank);
            }

            for (id, rank) in seen {
                // SAFETY: rank is typically < 1000, fits in f32 with full precision
                #[allow(clippy::cast_precision_loss)]
                let rrf_score = 1.0 / (k_f32 + (rank + 1) as f32);
                *doc_rrf.entry(id).or_insert(0.0) += rrf_score;
            }
        }

        let mut fused: Vec<(u64, f32)> = doc_rrf.into_iter().collect();
        fused.sort_by(|a, b| b.1.total_cmp(&a.1));

        Ok(fused)
    }

    /// Weighted fusion: combination of avg, max, and hit ratio.
    #[allow(clippy::cast_precision_loss)]
    fn fuse_weighted(
        results: Vec<Vec<(u64, f32)>>,
        avg_weight: f32,
        max_weight: f32,
        hit_weight: f32,
        total_queries: usize,
    ) -> Vec<(u64, f32)> {
        // Collect all scores per document
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

        // SAFETY: total_queries is typically < 100, fits in f32 with full precision
        #[allow(clippy::cast_precision_loss)]
        let total_q = total_queries as f32;

        let mut fused: Vec<(u64, f32)> = doc_scores
            .into_iter()
            .map(|(id, scores)| {
                // SAFETY: scores.len() is typically < 1000, fits in f32 with full precision
                #[allow(clippy::cast_precision_loss)]
                let avg = scores.iter().sum::<f32>() / scores.len() as f32;
                let max = scores.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                #[allow(clippy::cast_precision_loss)]
                let hit_ratio = scores.len() as f32 / total_q;

                let combined = avg_weight * avg + max_weight * max + hit_weight * hit_ratio;
                (id, combined)
            })
            .collect();

        fused.sort_by(|a, b| b.1.total_cmp(&a.1));

        fused
    }
}

impl Default for FusionStrategy {
    fn default() -> Self {
        Self::RRF { k: 60 }
    }
}
