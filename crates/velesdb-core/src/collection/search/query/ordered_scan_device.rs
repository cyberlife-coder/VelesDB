//! Retain the reported scores of a graph search without hydrating its full
//! result set. The ordered scan independently judges all rows, including
//! rows the approximate traversal never emitted.

use super::super::super::vector_filter::SearchBitmap;
use super::super::fetch_window::{fill_similarity_window, filtered_search_depth};
use super::super::options::QueryFinalizationContext;
use super::super::select_dispatch::main_select_execution_limit;
use super::super::similarity_filter::SimilarityLeaf;
use super::{Collection, QuerySearchOptions, Result, SearchResult, CHUNK};
use crate::scored_result::ScoredResult;

impl Collection {
    /// The same native prefix the SELECT would have read before sorting:
    /// graph-OR overfetch, CBO filter budget and live-row retries included.
    /// An anchored bitmap read uses its resolved graph ids in
    /// `ordered_anchor_hits`; a GraphFirst scan computes its scores on rows.
    pub(super) fn ordered_device_hits(
        &self,
        context: &QueryFinalizationContext<'_>,
        query: &[f32],
        opts: &QuerySearchOptions,
        fetch: usize,
    ) -> Result<Vec<ScoredResult>> {
        let extracted = context.extracted;
        let limit = main_select_execution_limit(extracted, fetch);
        if !extracted.similarity_conditions.is_empty() {
            let skip = Self::graph_or_reads_all_candidates(context.stmt, extracted);
            let metadata = Self::postfilter_metadata(extracted.filter_condition.as_ref(), skip);
            return self.threshold_near_hits(
                query,
                opts,
                &extracted.similarity_conditions,
                metadata.as_ref(),
                limit,
            );
        }
        if let Some(prepared) = &extracted.ordered_prefilter {
            let (_, overfetch) =
                self.compute_cbo_strategy(context.stmt, extracted.filter_condition.as_ref(), fetch);
            return self.search_projected_with_filter(
                query,
                filtered_search_depth(limit, overfetch),
                &prepared.filter,
                opts,
                SearchBitmap::Prepared(prepared.bitmap.as_ref()),
            );
        }
        self.search_projected_with_opts(query, limit, opts)
    }

    /// NEAR retains its native score; each WHERE threshold reads the
    /// original row vector. The cascade keeps only the last attempted
    /// prefix, as the row-returning dispatch does.
    fn threshold_near_hits(
        &self,
        query: &[f32],
        opts: &QuerySearchOptions,
        leaves: &[SimilarityLeaf],
        metadata: Option<&crate::filter::Filter>,
        limit: usize,
    ) -> Result<Vec<ScoredResult>> {
        fill_similarity_window(self.vector_count(), limit, leaves.len(), |k| {
            let candidates = self.search_projected_with_opts::<ScoredResult>(query, k, opts)?;
            let exhausted = candidates.len() < k;
            // The native threshold path limits its rows at the metadata
            // postfilter; without one it returns the whole passing prefix.
            let max_rows = if metadata.is_some() {
                limit
            } else {
                usize::MAX
            };
            let mut passing = Vec::new();
            for chunk in candidates.chunks(CHUNK) {
                if passing.len() >= max_rows {
                    break;
                }
                self.extend_threshold_hits(&mut passing, chunk, leaves, metadata, max_rows);
            }
            Ok((passing, exhausted))
        })
    }

    fn extend_threshold_hits(
        &self,
        passing: &mut Vec<ScoredResult>,
        chunk: &[ScoredResult],
        leaves: &[SimilarityLeaf],
        metadata: Option<&crate::filter::Filter>,
        max_rows: usize,
    ) {
        let ids: Vec<_> = chunk.iter().map(|hit| hit.id).collect();
        let rows = self
            .get(&ids)
            .into_iter()
            .zip(chunk)
            .filter_map(|(point, hit)| Some(SearchResult::new(point?, hit.score)))
            .collect();
        let passing_rows = self.filter_by_similarity(rows, leaves, usize::MAX);
        passing.extend(
            passing_rows
                .into_iter()
                .map(|(row, _)| row)
                .filter(|row| {
                    metadata
                        .is_none_or(|f| f.matches_point(row.point.id, row.point.payload.as_ref()))
                })
                .take(max_rows.saturating_sub(passing.len()))
                .map(|row| ScoredResult::new(row.point.id, row.score)),
        );
    }
}
