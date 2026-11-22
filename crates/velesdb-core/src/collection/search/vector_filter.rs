//! Filter selectivity estimation and filtered search dispatch.
//!
//! Extracted from `vector.rs` to reduce NLOC.

// Reason: Numeric casts in selectivity estimation are intentional:
// - usize->f64 for selectivity ratios: values are small counts
// - f64->usize for oversampled k: saturating, then clamped to [k+10, max(k+10, 10_000)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]

use crate::collection::search::live::{LiveHit, Nearest};
use crate::collection::search::pending::PendingRestriction;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::scored_result::ScoredResult;
use crate::velesql::{decide_filter_strategy, FilterDecisionMode, FilterStrategy};

/// An ordered query already looked for this filter's bitmap while planning.
pub(super) enum SearchBitmap<'a> {
    Lookup,
    Prepared(Option<&'a roaring::RoaringBitmap>),
}

impl<'a> SearchBitmap<'a> {
    pub(super) fn resolve(
        self,
        col: &Collection,
        filter: &crate::filter::Filter,
    ) -> Option<std::borrow::Cow<'a, roaring::RoaringBitmap>> {
        match self {
            Self::Lookup => col
                .build_prefilter_bitmap(filter)
                .map(std::borrow::Cow::Owned),
            Self::Prepared(bitmap) => bitmap.map(std::borrow::Cow::Borrowed),
        }
    }
}

/// How a filtered search with quality overrides asks the index for its
/// candidates, chosen once from the filter's prefilter bitmap.
#[derive(Clone, Copy)]
enum FilteredIndexSearch<'b> {
    /// No bitmap, or one too unselective to restrict by: the quality-aware
    /// HNSW search, every candidate post-filtered.
    PostFilter,
    /// An exact scan of the bitmap's points.
    BitmapScan(&'b roaring::RoaringBitmap),
    /// The HNSW search restricted to the bitmap's points.
    BitmapHnsw(&'b roaring::RoaringBitmap),
}

impl FilteredIndexSearch<'_> {
    /// `want`, its index search restricted as this one is: a bitmap path
    /// reaches the bitmap's members alone, the post-filter path every stored
    /// vector.
    fn restrict(self, want: Nearest<'_>) -> Nearest<'_> {
        match self {
            Self::PostFilter => want,
            Self::BitmapScan(bitmap) | Self::BitmapHnsw(bitmap) => want.within(bitmap.len()),
        }
    }
}

impl Collection {
    /// Searches with metadata filtering AND quality options from a WITH clause.
    ///
    /// The `k` nearest live rows the filter matches, among as many live
    /// candidates as the chosen index search calls for
    /// (`collection/search/live.rs`). The filter re-matches every candidate
    /// even on the bitmap paths: the prefilter bitmap can be a SUPERSET of
    /// the true matches (non-indexed conditions, mirror false positives),
    /// and the post-filter path merges every pending point unfiltered (see
    /// `pending.rs`) — skipping it would return false positives.
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
        self.search_projected_with_filter(query, k, filter, opts, SearchBitmap::Lookup)
    }

    pub(super) fn search_projected_with_filter<T: LiveHit>(
        &self,
        query: &[f32],
        k: usize,
        filter: &crate::filter::Filter,
        opts: &crate::collection::search::query::QuerySearchOptions,
        prepared: SearchBitmap<'_>,
    ) -> Result<Vec<T>> {
        if !opts.has_quality_overrides() {
            return self.search_with_filter_impl(query, k, filter, Some(opts), prepared);
        }

        let config = self.storage.config.read();
        config.query_vector_target().check_len(query.len())?;
        let metric = config.metric;
        drop(config);

        let quality = opts.resolved_quality(self.runtime_search_quality());

        // Parity item E: gate Perfect-mode over-cap before any index dispatch
        // so a filtered `WITH (mode='perfect')` query cannot trigger an
        // unbounded brute-force scan, matching the unfiltered entry points.
        self.enforce_perfect_mode_limit(quality)?;

        let bitmap = prepared.resolve(self, filter);
        let search = match bitmap.as_deref() {
            Some(bitmap) if bitmap.is_empty() => {
                // The bitmap answers the query exactly: nothing matches.
                opts.record_executed_strategy(FilterStrategy::PreFilterExact);
                return Ok(Vec::new());
            }
            Some(bitmap) => {
                let (strategy, search) = self.bitmap_search(bitmap);
                opts.record_executed_strategy(strategy);
                search
            }
            None => {
                opts.record_executed_strategy(FilterStrategy::PostFilter);
                FilteredIndexSearch::PostFilter
            }
        };
        let budget = match search {
            FilteredIndexSearch::BitmapScan(_) => k,
            _ => compute_oversampled_k(k, filter, Some(&self.get_stats())),
        };
        self.nearest_live_where(
            &search.restrict(Nearest::judging(query, metric, k, budget)),
            |id, payload| filter.matches_point(id, payload),
            |n| self.run_filtered_index_search(search, query, n, quality, metric),
        )
    }

    /// [`Self::search_with_filter_and_opts`] for callers that carry a bare
    /// [`SearchQuality`](crate::SearchQuality) rather than a parsed `WITH`
    /// clause — the read gate's filtered leaf (`run_dense`), which maps an
    /// `ef` override to `Custom` before calling. Building the options here
    /// keeps `QuerySearchOptions` construction inside this module; the
    /// recorded-strategy probe stays `None` outside the query pipeline.
    ///
    /// # Errors
    ///
    /// Returns an error if the query vector dimension doesn't match the
    /// collection, or the [`SearchQuality::Perfect`](crate::SearchQuality)
    /// guard rail fires.
    pub(crate) fn search_with_quality_and_filter(
        &self,
        query: &[f32],
        k: usize,
        quality: crate::SearchQuality,
        filter: &crate::filter::Filter,
    ) -> Result<Vec<SearchResult>> {
        let opts = crate::collection::search::query::QuerySearchOptions {
            quality: Some(quality),
            ..Default::default()
        };
        self.search_with_filter_and_opts(query, k, filter, &opts)
    }

    /// The scorer of a full-depth restricted search on this bitmap.
    pub(super) fn filtered_row_scores(
        &self,
        bitmap: &roaring::RoaringBitmap,
        opts: &crate::collection::search::query::QuerySearchOptions,
        quality: crate::SearchQuality,
    ) -> crate::index::hnsw::index::IndexRowScores {
        use crate::index::hnsw::index::IndexRowScores;
        let native = self.storage.index.full_depth_row_scores(quality, None);
        let search = if opts.has_quality_overrides() {
            self.bitmap_search(bitmap).1
        } else {
            FilteredIndexSearch::BitmapHnsw(bitmap)
        };
        match search {
            FilteredIndexSearch::BitmapScan(_) => IndexRowScores::Bitmap,
            FilteredIndexSearch::BitmapHnsw(_) => self.storage.index.bitmap_row_scores(quality),
            FilteredIndexSearch::PostFilter => native,
        }
    }

    /// Chooses the strategy and index search for a non-empty prefilter bitmap.
    /// The dispatch site records the strategy when it runs that search.
    fn bitmap_search<'b>(
        &self,
        bitmap: &'b roaring::RoaringBitmap,
    ) -> (FilterStrategy, FilteredIndexSearch<'b>) {
        let selectivity =
            super::vector::estimate_real_selectivity(bitmap, self.storage.index.len());
        let strategy = decide_filter_strategy(selectivity, FilterDecisionMode::Exact, None);
        let search = match strategy {
            FilterStrategy::PostFilter => FilteredIndexSearch::PostFilter,
            FilterStrategy::PreFilterExact => FilteredIndexSearch::BitmapScan(bitmap),
            // Exact mode never emits `None`; if it ever did, the bitmap
            // path below is the shape that ran. Spelled out (no wildcard)
            // so a future `FilterStrategy` variant fails compilation here
            // and forces an explicit dispatch choice.
            FilterStrategy::PreFilter | FilterStrategy::None => {
                FilteredIndexSearch::BitmapHnsw(bitmap)
            }
        };
        (strategy, search)
    }

    /// Asks the index for its `n` nearest candidates by `search`, the
    /// pending points merged under the matching restriction.
    fn run_filtered_index_search(
        &self,
        search: FilteredIndexSearch<'_>,
        query: &[f32],
        n: usize,
        quality: crate::SearchQuality,
        metric: crate::DistanceMetric,
    ) -> Result<Vec<ScoredResult>> {
        let index = &self.storage.index;
        match search {
            FilteredIndexSearch::PostFilter => self.search_merging_pending(
                query,
                n,
                metric,
                PendingRestriction::post_filtered(),
                |m| index.search_with_quality(query, m, quality),
            ),
            FilteredIndexSearch::BitmapScan(bitmap) => self.search_merging_pending(
                query,
                n,
                metric,
                PendingRestriction::within(bitmap).and_post_filtered(),
                |m| index.full_scan_with_bitmap(query, m, bitmap),
            ),
            FilteredIndexSearch::BitmapHnsw(bitmap) => self.search_merging_pending(
                query,
                n,
                metric,
                PendingRestriction::within(bitmap).and_post_filtered(),
                |m| index.search_with_quality_and_bitmap(query, m, quality, bitmap),
            ),
        }
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

/// The most candidates a filter's oversampling asks HNSW for beyond the
/// points wanted.
const OVERSAMPLE_CAP: usize = 10_000;

/// Computes the oversampled candidate count for filtered search.
///
/// `k / selectivity` candidates, so the filter leaves `k` of them, bounded to
/// `[k + 10, max(k + 10, 10_000)]`: the ceiling bounds the oversampling, never
/// the `k` points asked for — a search for more than 10 000 points whose
/// filter passes them all still finds `k`. The bounds stay ordered for any
/// `k`, so the clamp cannot panic.
pub(super) fn compute_oversampled_k(
    k: usize,
    filter: &crate::filter::Filter,
    stats: Option<&crate::collection::stats::CollectionStats>,
) -> usize {
    // Clamp to a tiny positive value so that a zero-selectivity filter (e.g. empty
    // IN clause) never produces NaN (0.0/0.0 when k=0) or Inf (k>0/0.0); the
    // float-to-int cast below saturates an infinite quotient to `usize::MAX`.
    let selectivity = stats
        .map_or_else(
            || estimate_filter_selectivity(filter),
            |s| s.estimate_runtime_filter_selectivity(filter),
        )
        .max(1e-9);
    let floor = k.saturating_add(10);
    let ceiling = OVERSAMPLE_CAP.max(floor);
    let wanted = (k as f64 / selectivity).ceil() as usize;
    wanted.clamp(floor, ceiling)
}

/// Heuristic selectivity estimate based on filter structure.
fn estimate_filter_selectivity(filter: &crate::filter::Filter) -> f64 {
    estimate_condition_selectivity(&filter.condition)
}

/// Structure-only fallback used when no [`CollectionStats`] is available
/// (raw search paths). Constants come from the shared
/// [`selectivity_defaults`](crate::collection::stats::selectivity_defaults)
/// table so this path cannot drift from the stats-backed one.
///
/// [`CollectionStats`]: crate::collection::stats::CollectionStats
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
