//! Filter selectivity estimation and filtered search dispatch.
//!
//! Extracted from `vector.rs` to reduce NLOC.

// Reason: Numeric casts in selectivity estimation are intentional:
// - usize->f64 for selectivity ratios: values are small counts
// - f64->usize for clamped oversampled k: result is bounded to [min(k+10, 10_000), 10_000]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::expiry::{is_payload_expired, now_unix_secs};
use crate::collection::search::resolve;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::scored_result::ScoredResult;
use crate::storage::{PayloadStorage, VectorStorage};
use crate::validation::validate_dimension_match;
use crate::velesql::{decide_filter_strategy, FilterDecisionMode, FilterStrategy};

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
            return self.search_with_filter_recorded(query, k, filter, opts);
        }

        let config = self.storage.config.read();
        validate_dimension_match(config.dimension, query.len())?;
        let higher_is_better = config.metric.higher_is_better();
        let metric = config.metric;
        drop(config);

        let quality = resolve_quality(opts);

        // Parity item E: gate Perfect-mode over-cap before any index dispatch
        // so a filtered `WITH (mode='perfect')` query cannot trigger an
        // unbounded brute-force scan, matching the unfiltered entry points.
        self.enforce_perfect_mode_limit(quality)?;

        let index_results = match self.build_prefilter_bitmap(filter) {
            Some(bitmap) if bitmap.is_empty() => {
                // The bitmap answers the query exactly: nothing matches.
                opts.record_executed_strategy(FilterStrategy::PreFilterExact);
                return Ok(Vec::new());
            }
            Some(bitmap) => {
                self.search_with_bitmap_strategy(query, k, filter, quality, metric, &bitmap, opts)?
            }
            None => {
                opts.record_executed_strategy(FilterStrategy::PostFilter);
                self.search_post_filter(query, k, filter, quality, metric)?
            }
        };

        // The full re-match inside `filter_and_hydrate` is deliberate even on
        // the bitmap path: the prefilter bitmap can be a SUPERSET of the true
        // matches (non-indexed conditions, mirror false positives) and never
        // covers delta-buffer points — skipping it would return false positives.
        Ok(self.filter_and_hydrate(index_results, filter, k, higher_is_better))
    }

    /// Dispatches to full-scan, HNSW+bitmap, or post-filter based on selectivity.
    #[allow(clippy::too_many_arguments)] // Reason: dispatch bundle mirrors the query's full shape.
    fn search_with_bitmap_strategy(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
        quality: crate::SearchQuality,
        metric: crate::DistanceMetric,
        bitmap: &roaring::RoaringBitmap,
        opts: &crate::collection::search::query::QuerySearchOptions,
    ) -> Result<Vec<ScoredResult>> {
        let selectivity =
            super::vector::estimate_real_selectivity(bitmap, self.storage.index.len());

        let strategy = decide_filter_strategy(selectivity, FilterDecisionMode::Exact, None);
        opts.record_executed_strategy(strategy);
        match strategy {
            FilterStrategy::PostFilter => {
                self.search_post_filter(query, k, filter, quality, metric)
            }
            FilterStrategy::PreFilterExact => {
                let results = self.storage.index.full_scan_with_bitmap(query, k, bitmap)?;
                Ok(self.merge_delta(results, query, k, metric))
            }
            // Exact mode never emits `None`; if it ever did, the bitmap
            // path below is the shape that ran. Spelled out (no wildcard)
            // so a future `FilterStrategy` variant fails compilation here
            // and forces an explicit dispatch choice.
            FilterStrategy::PreFilter | FilterStrategy::None => {
                let candidates_k = compute_oversampled_k(k, filter, Some(&self.get_stats()));
                let results = self.storage.index.search_with_quality_and_bitmap(
                    query,
                    candidates_k,
                    quality,
                    bitmap,
                )?;
                Ok(self.merge_delta(results, query, candidates_k, metric))
            }
        }
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
        let candidates_k = compute_oversampled_k(k, filter, Some(&self.get_stats()));
        let index_results = self
            .storage
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
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let now_secs = now_unix_secs();

        // Phase 1 — filter on payloads only. The pre-fix shape hydrated the
        // vector of EVERY candidate that passed the filter (the oversampled
        // budget reaches 10 000), then threw all but k away: up to ~60 MB of
        // vector copies per 1536-dim query to keep 10.
        let mut passing: Vec<(u64, f32, Option<serde_json::Value>)> = index_results
            .into_iter()
            .filter_map(|sr| {
                let payload = payload_storage.retrieve(sr.id).ok().flatten();
                if is_payload_expired(payload.as_ref(), now_secs) {
                    return None;
                }
                // Correctness-bearing re-match (see dispatch above): candidates
                // may come from an over-approximate bitmap or the delta buffer.
                let matches = match payload.as_ref() {
                    Some(p) => filter.matches(p),
                    None => filter.matches(&serde_json::Value::Null),
                };
                matches.then_some((sr.id, sr.score, payload))
            })
            .collect();
        // Phase 1 read every payload it needs; phase 2 touches vectors only.
        drop(payload_storage);

        resolve::sort_scored_ids_by_metric(&mut passing, higher_is_better);

        // Phase 2 — hydrate vectors in sorted order until k results stand. A
        // candidate whose vector is missing is skipped and the next one takes
        // its place, exactly as the eager filter_map dropped it pre-truncate.
        let mut results: Vec<SearchResult> = Vec::with_capacity(k.min(passing.len()));
        for (id, score, payload) in passing {
            if results.len() >= k {
                break;
            }
            let Some(vector) = vector_storage.retrieve(id).ok().flatten() else {
                continue;
            };
            results.push(SearchResult::new(
                crate::point::Point {
                    id,
                    vector,
                    payload,
                    sparse_vectors: None,
                },
                score,
            ));
        }
        drop(vector_storage);
        super::vector::tag_vector_component_scores(&mut results);
        results
    }
}

/// Maps an explicit `ef_search` value to a value-preserving `SearchQuality`.
///
/// Uses `Custom(ef)` so the exact budget is honored (the documented contract
/// for `WITH (ef_search = N)`), instead of snapping to a coarse named profile.
#[must_use]
pub(crate) fn ef_to_quality(ef_search: usize) -> crate::SearchQuality {
    crate::SearchQuality::Custom(ef_search)
}

/// Resolves the search quality from query options.
fn resolve_quality(
    opts: &crate::collection::search::query::QuerySearchOptions,
) -> crate::SearchQuality {
    opts.quality.unwrap_or_else(|| {
        opts.ef_search
            .map_or(crate::SearchQuality::Balanced, ef_to_quality)
    })
}

/// Hard upper bound on the oversampled HNSW candidate budget.
const OVERSAMPLE_CAP: f64 = 10_000.0;

/// Computes the oversampled candidate count for filtered search.
///
/// The result is bounded to `[min(k + 10, 10_000), 10_000]`: the candidate
/// budget saturates at the cap for any `k >= 9_990`, so callers requesting
/// huge `k` (e.g. LIMIT close to `MAX_LIMIT`) get at most 10_000 candidates
/// instead of panicking — `f64::clamp` asserts `min <= max`, and an
/// unbounded lower bound of `k + 10` used to violate that for large `k`.
pub(super) fn compute_oversampled_k(
    k: usize,
    filter: &crate::filter::Filter,
    stats: Option<&crate::collection::stats::CollectionStats>,
) -> usize {
    // Clamp to a tiny positive value so that a zero-selectivity filter (e.g. empty
    // IN clause) never produces NaN (0.0/0.0 when k=0) or Inf (k>0/0.0). Both would
    // be handled by the clamp below, but NaN→usize is implementation-defined (LLVM
    // saturates to 0, giving zero candidates instead of the minimum sensible count).
    let selectivity = stats
        .map_or_else(
            || estimate_filter_selectivity(filter),
            |s| s.estimate_runtime_filter_selectivity(filter),
        )
        .max(1e-9);
    #[allow(clippy::cast_precision_loss)]
    let k_f64 = k as f64;
    #[allow(clippy::cast_precision_loss)]
    let lower = ((k.saturating_add(10)) as f64).min(OVERSAMPLE_CAP);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let clamped = (k_f64 / selectivity).ceil().clamp(lower, OVERSAMPLE_CAP) as usize;
    clamped
}

/// Heuristic selectivity estimate based on filter structure.
fn estimate_filter_selectivity(filter: &crate::filter::Filter) -> f64 {
    estimate_condition_selectivity(&filter.condition)
}

/// Structure-only fallback used when no [`CollectionStats`] is available
/// (raw search paths). Constants come from the shared
/// [`selectivity_defaults`](crate::collection::stats::selectivity_defaults)
/// table so this path cannot drift from the stats-backed one.
fn estimate_condition_selectivity(cond: &crate::filter::Condition) -> f64 {
    use crate::collection::stats::selectivity_defaults as d;
    use crate::filter::Condition;
    match cond {
        Condition::Eq { .. } | Condition::IsNull { .. } => d::EQ,
        Condition::Gt { .. }
        | Condition::Gte { .. }
        | Condition::Lt { .. }
        | Condition::Lte { .. }
        | Condition::Contains { .. }
        | Condition::Like { .. }
        | Condition::ILike { .. }
        | Condition::ArrayContains { .. }
        | Condition::ArrayContainsAny { .. }
        | Condition::ArrayContainsAll { .. }
        | Condition::GeoDistance { .. }
        | Condition::GeoBbox { .. } => d::RANGE,
        Condition::In { values, .. } => {
            #[allow(clippy::cast_precision_loss)]
            let sel = values.len() as f64 * d::IN_PER_VALUE;
            sel.min(d::IN_CAP)
        }
        Condition::Neq { .. } | Condition::IsNotNull { .. } => d::NEGATION,
        Condition::And { conditions } => conditions
            .iter()
            .map(estimate_condition_selectivity)
            .product::<f64>()
            .max(d::FLOOR),
        Condition::Or { conditions } => conditions
            .iter()
            .map(estimate_condition_selectivity)
            .sum::<f64>()
            .min(1.0),
        Condition::Not { condition } => {
            (1.0 - estimate_condition_selectivity(condition)).max(d::FLOOR)
        }
    }
}
