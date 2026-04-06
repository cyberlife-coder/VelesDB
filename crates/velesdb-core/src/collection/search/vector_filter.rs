//! Filter selectivity estimation and filtered search dispatch.
//!
//! Extracted from `vector.rs` to reduce NLOC.

// SAFETY: Numeric casts in selectivity estimation are intentional:
// - usize->f64 for selectivity ratios: values are small counts
// - f64->usize for clamped oversampled k: result is bounded to [k+10, 10_000]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::search::resolve;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::scored_result::ScoredResult;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;

/// Selectivity threshold below which full-scan brute-force is used.
pub(crate) const SELECTIVITY_THRESHOLD: f64 = 0.01;
/// Selectivity threshold above which bitmap is skipped in favor of post-filter.
const SELECTIVITY_HIGH_THRESHOLD: f64 = 0.8;

impl Collection {
    /// Searches with metadata filtering AND quality options from a WITH clause.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the collection.
    pub(crate) fn search_with_filter_and_opts(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
        opts: &crate::collection::search::query::QuerySearchOptions,
    ) -> Result<Vec<SearchResult>> {
        if !opts.has_quality_overrides() {
            return self.search_with_filter(query, k, filter);
        }

        let config = self.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let higher_is_better = config.metric.higher_is_better();
        let metric = config.metric;
        drop(config);

        let quality = resolve_quality(opts);

        let index_results = match self.build_prefilter_bitmap(filter) {
            Some(bitmap) if bitmap.is_empty() => return Ok(Vec::new()),
            Some(bitmap) => {
                self.search_with_bitmap_strategy(query, k, filter, quality, metric, &bitmap)?
            }
            None => self.search_post_filter(query, k, filter, quality, metric)?,
        };

        Ok(self.filter_and_hydrate(index_results, filter, k, higher_is_better))
    }

    /// Dispatches to full-scan, HNSW+bitmap, or post-filter based on selectivity.
    fn search_with_bitmap_strategy(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
        quality: crate::SearchQuality,
        metric: crate::DistanceMetric,
        bitmap: &roaring::RoaringBitmap,
    ) -> Result<Vec<ScoredResult>> {
        let selectivity = super::vector::estimate_real_selectivity(bitmap, self.index.len());

        if selectivity > SELECTIVITY_HIGH_THRESHOLD {
            return self.search_post_filter(query, k, filter, quality, metric);
        }

        if selectivity <= SELECTIVITY_THRESHOLD {
            let results = self.index.full_scan_with_bitmap(query, k, bitmap)?;
            return Ok(self.merge_delta(results, query, k, metric));
        }

        let candidates_k = compute_oversampled_k(k, filter);
        let results =
            self.index
                .search_with_quality_and_bitmap(query, candidates_k, quality, bitmap)?;
        Ok(self.merge_delta(results, query, candidates_k, metric))
    }

    /// Searches without bitmap pre-filter, using quality-aware HNSW + post-filter.
    fn search_post_filter(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
        quality: crate::SearchQuality,
        metric: crate::DistanceMetric,
    ) -> Result<Vec<ScoredResult>> {
        let candidates_k = compute_oversampled_k(k, filter);
        let index_results = self
            .index
            .search_with_quality(query, candidates_k, quality)?;
        Ok(self.merge_delta(index_results, query, candidates_k, metric))
    }

    /// Filters scored results by metadata and hydrates matching points.
    pub(super) fn filter_and_hydrate(
        &self,
        index_results: Vec<ScoredResult>,
        filter: &crate::filter::Filter,
        k: usize,
        higher_is_better: bool,
    ) -> Vec<SearchResult> {
        let vector_storage = self.vector_storage.read();
        let payload_storage = self.payload_storage.read();

        let mut results: Vec<SearchResult> = index_results
            .into_iter()
            .filter_map(|sr| {
                let payload = payload_storage.retrieve(sr.id).ok().flatten();
                let matches = match payload.as_ref() {
                    Some(p) => filter.matches(p),
                    None => filter.matches(&serde_json::Value::Null),
                };
                if !matches {
                    return None;
                }
                let vector = vector_storage.retrieve(sr.id).ok().flatten()?;
                Some(SearchResult::new(
                    crate::point::Point {
                        id: sr.id,
                        vector,
                        payload,
                        sparse_vectors: None,
                    },
                    sr.score,
                ))
            })
            .collect();

        resolve::sort_results_by_metric(&mut results, higher_is_better);
        results.truncate(k);
        super::vector::tag_vector_component_scores(&mut results);
        results
    }
}

/// Resolves the search quality from query options.
fn resolve_quality(
    opts: &crate::collection::search::query::QuerySearchOptions,
) -> crate::SearchQuality {
    opts.quality.unwrap_or_else(|| {
        opts.ef_search
            .map_or(crate::SearchQuality::Balanced, |ef| match ef {
                0..=64 => crate::SearchQuality::Fast,
                65..=128 => crate::SearchQuality::Balanced,
                129..=512 => crate::SearchQuality::Accurate,
                _ => crate::SearchQuality::Perfect,
            })
    })
}

/// Computes the oversampled candidate count for filtered search.
pub(super) fn compute_oversampled_k(k: usize, filter: &crate::filter::Filter) -> usize {
    let selectivity = estimate_filter_selectivity(filter);
    #[allow(clippy::cast_precision_loss)]
    let k_f64 = k as f64;
    #[allow(clippy::cast_precision_loss)]
    let lower = (k + 10) as f64;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let clamped = (k_f64 / selectivity).ceil().clamp(lower, 10_000.0) as usize;
    clamped
}

/// Heuristic selectivity estimate based on filter structure.
fn estimate_filter_selectivity(filter: &crate::filter::Filter) -> f64 {
    estimate_condition_selectivity(&filter.condition)
}

fn estimate_condition_selectivity(cond: &crate::filter::Condition) -> f64 {
    use crate::filter::Condition;
    match cond {
        Condition::Eq { .. } | Condition::IsNull { .. } => 0.1,
        Condition::Gt { .. }
        | Condition::Gte { .. }
        | Condition::Lt { .. }
        | Condition::Lte { .. }
        | Condition::Contains { .. }
        | Condition::Like { .. }
        | Condition::ILike { .. }
        | Condition::ArrayContains { .. }
        | Condition::ArrayContainsAny { .. }
        | Condition::ArrayContainsAll { .. } => 0.3,
        Condition::In { values, .. } => {
            #[allow(clippy::cast_precision_loss)]
            let sel = values.len() as f64 * 0.05;
            sel.min(0.8)
        }
        Condition::Neq { .. } | Condition::IsNotNull { .. } => 0.9,
        Condition::And { conditions } => conditions
            .iter()
            .map(estimate_condition_selectivity)
            .product::<f64>()
            .max(0.01),
        Condition::Or { conditions } => conditions
            .iter()
            .map(estimate_condition_selectivity)
            .sum::<f64>()
            .min(1.0),
        Condition::Not { condition } => (1.0 - estimate_condition_selectivity(condition)).max(0.01),
    }
}
