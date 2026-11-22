//! The ordered scan ranks the rows a SELECT admits by its `ORDER BY` and
//! returns the leading `OFFSET + LIMIT` rows for final pagination.
//!
//! It is the read of every `ORDER BY` the fetch does not already rank by,
//! when every score its rows carry is one a scan computes
//! ([`FetchWindow::OrderedScan`](super::fetch_window::FetchWindow)). Rows
//! are admitted as the exact `similarity()` scan admits them
//! ([`Collection::scan_candidate`]): every row the SELECT reads
//! ([`RowUniverse`], only those storing a vector under a `NEAR`), none that
//! has expired, each decided by the one WHERE evaluator — thresholds
//! three-valued, graph predicates by their anchor sets — and scored as the
//! route it stands in for scores it (a `NEAR` by its vector, otherwise the
//! first `similarity()` leaf, otherwise the metadata score).
//!
//! The plan holds all candidate ids. [`LeadingRows`] keeps
//! O(window size + chunk size) hydrated rows and uses the `ORDER BY`
//! comparator ([`Collection::rank_rows`]) to narrow them after each batch
//! that crosses its buffer threshold. Each chunk reads storage under its
//! own guards. Ranking uses the primary and named vectors on each hydrated
//! row. Concurrent writes can be visible between batches.

use std::collections::{HashMap, HashSet};

use super::graph_prefilter::OrderedAnchors;
use super::options::{OrderedIndexScores, QueryFinalizationContext};
use super::row_universe::{RowScope, RowUniverse};
use super::similarity_scan::{RankingLeaf, RowScore, ScanCtx};
use super::{ExtractedComponents, QuerySearchOptions};
use crate::collection::expiry::now_unix_secs;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{LetBinding, SelectOrderBy, SelectStatement};

/// Ids read per hold of the storage guards.
const CHUNK: usize = 4096;

/// Buffers hydrated rows until a batch takes the count past the larger of
/// twice the window and [`CHUNK`], then retains the leading window. With
/// stable sort keys, the total order gives the same window as a full sort.
struct LeadingRows<'a> {
    collection: &'a Collection,
    order_by: &'a [SelectOrderBy],
    params: &'a HashMap<String, serde_json::Value>,
    let_bindings: &'a [LetBinding],
    leading: usize,
    rows: Vec<SearchResult>,
}

impl LeadingRows<'_> {
    /// Takes `rows` in, narrowing to the window once too many are held.
    fn extend(&mut self, rows: Vec<SearchResult>) -> Result<()> {
        self.rows.extend(rows);
        if self.rows.len() > self.leading.saturating_mul(2).max(CHUNK) {
            self.narrow()?;
        }
        Ok(())
    }

    /// Keeps the leading rows only, scores unchanged.
    fn narrow(&mut self) -> Result<()> {
        let per_result_let = if self.let_bindings.is_empty() {
            Vec::new()
        } else {
            Collection::evaluate_let_for_results(self.let_bindings, &self.rows)
        };
        self.collection.rank_rows(
            &mut self.rows,
            self.order_by,
            self.params,
            &per_result_let,
            self.leading,
        )?;
        self.rows.truncate(self.leading);
        Ok(())
    }

    /// The window's rows, in no defined order.
    fn into_rows(mut self) -> Result<Vec<SearchResult>> {
        self.narrow()?;
        Ok(self.rows)
    }
}

/// What an ordered scan reads and how it scores each row.
struct ScanPlan {
    anchors: Option<OrderedAnchors>,
    ids: Vec<u64>,
    universe: RowUniverse,
    leaf: Option<RankingLeaf>,
    near: Option<Vec<f32>>,
}

/// Primary scores that use an index representation of the row, after WHERE
/// has judged the original. Pending rows take their current stored vector.
struct NativeScanScores<'a> {
    anchors: Option<&'a OrderedAnchors>,
    query: &'a [f32],
    pending: super::super::pending::PendingSnapshot,
    metric: crate::DistanceMetric,
    index: Option<crate::index::hnsw::index::IndexRowScores>,
    pq: bool,
    #[cfg(any(feature = "gpu", test))]
    emitted: Option<std::iter::Peekable<std::vec::IntoIter<crate::scored_result::ScoredResult>>>,
}

#[cfg(any(feature = "gpu", test))]
impl NativeScanScores<'_> {
    fn keep_emitted(&mut self, mut hits: Vec<crate::scored_result::ScoredResult>) {
        hits.sort_unstable_by_key(|hit| hit.id);
        self.emitted = Some(hits.into_iter().peekable());
    }

    fn emitted_score(&mut self, id: u64) -> Option<f32> {
        let hits = self.emitted.as_mut()?;
        while hits.next_if(|hit| hit.id < id).is_some() {}
        hits.next_if(|hit| hit.id == id).map(|hit| hit.score)
    }
}

impl ScanPlan {
    fn native_index(
        &self,
        collection: &Collection,
        extracted: &ExtractedComponents,
    ) -> Option<crate::index::hnsw::index::IndexRowScores> {
        match extracted.ordered_index_scores {
            Some(OrderedIndexScores::Anchors) => self.anchors.as_ref().map(|_| {
                collection
                    .storage
                    .index
                    .bitmap_row_scores(collection.runtime_search_quality())
            }),
            Some(OrderedIndexScores::Index(source)) => Some(source),
            None => None,
        }
    }

    fn score(&self) -> RowScore<'_> {
        match (&self.near, &self.leaf) {
            (Some(near), _) => RowScore::Near(near),
            (None, Some(leaf)) => RowScore::Leaf(leaf),
            (None, None) => RowScore::Unscored,
        }
    }
}

impl Collection {
    /// Runs the SELECT by the ordered scan, then its
    /// post-processing and guard-rails ([`Self::finalize_query_results`]).
    ///
    /// # Errors
    ///
    /// An unresolvable vector, an evaluation error (a nested MATCH), or a
    /// guard-rail violation.
    pub(super) fn execute_ordered_scan(
        &self,
        finalization: &QueryFinalizationContext<'_>,
        search_opts: &QuerySearchOptions,
        pq_scores: bool,
    ) -> Result<Vec<SearchResult>> {
        if finalization.extracted.requests_dense_search() {
            let quality = search_opts.resolved_quality(self.runtime_search_quality());
            self.enforce_perfect_mode_limit(quality)?;
        }
        if !finalization.let_bindings.is_empty() {
            Self::validate_let_binding_support(finalization.extracted)?;
        }
        let mut results = self
            .ordered_scan_rows(finalization, search_opts, pq_scores)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        self.finalize_query_results(&mut results, finalization)?;
        Ok(results)
    }

    /// The rows of the window of `stmt` — its leading `OFFSET + limit` rows
    /// by its `ORDER BY` among every row it admits — in no defined order.
    fn ordered_scan_rows(
        &self,
        context: &QueryFinalizationContext<'_>,
        opts: &QuerySearchOptions,
        pq_scores: bool,
    ) -> Result<Vec<SearchResult>> {
        let stmt = context.stmt;
        let offset = stmt
            .offset
            .map_or(0, |offset| usize::try_from(offset).unwrap_or(usize::MAX));
        let mut window = LeadingRows {
            collection: self,
            order_by: stmt.order_by.as_deref().unwrap_or_default(),
            params: context.params,
            let_bindings: context.let_bindings,
            leading: offset.saturating_add(context.limit),
            rows: Vec::new(),
        };
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let plan = self.ordered_scan_plan::<{ Self::SIMILARITY_SCAN_MAX }>(
            stmt,
            context.params,
            context.extracted,
            &mut graph_cache,
            window.leading,
        )?;
        let mut native =
            self.native_scan_scores(&plan, context.extracted, pq_scores, window.leading);
        #[cfg(feature = "gpu")]
        if let Some(scores) = &mut native {
            self.load_ordered_graph_hits(scores, &plan, context, opts)?;
        }
        #[cfg(not(feature = "gpu"))]
        let _ = opts;
        let now_secs = now_unix_secs();
        for chunk in plan.ids.chunks(CHUNK) {
            let mut rows = self.scan_chunk(chunk, context, &plan, now_secs, &mut graph_cache)?;
            if let Some(scores) = &mut native {
                self.apply_native_scan_scores(&mut rows, scores);
            }
            window.extend(rows)?;
        }
        window.into_rows()
    }

    fn native_scan_scores<'q>(
        &self,
        plan: &'q ScanPlan,
        extracted: &ExtractedComponents,
        pq: bool,
        leading: usize,
    ) -> Option<NativeScanScores<'q>> {
        if leading == 0 || plan.ids.is_empty() {
            return None;
        }
        let index = plan.native_index(self, extracted);
        let query = plan.near.as_deref().filter(|_| pq || index.is_some())?;
        Some(NativeScanScores {
            anchors: plan.anchors.as_ref(),
            query,
            pending: self.snapshot_pending(),
            metric: self.storage.config.read().metric,
            index,
            pq,
            #[cfg(any(feature = "gpu", test))]
            emitted: None,
        })
    }

    /// A compound order ranks one relation. Capture its native graph scores
    /// at the same reference depth for every page, respecting an anchored bitmap.
    #[cfg(feature = "gpu")]
    fn load_ordered_graph_hits(
        &self,
        scores: &mut NativeScanScores<'_>,
        plan: &ScanPlan,
        context: &QueryFinalizationContext<'_>,
        opts: &QuerySearchOptions,
    ) -> Result<()> {
        if scores.index == Some(crate::index::hnsw::index::IndexRowScores::Graph)
            && self.storage.index.has_graph_score_device()
        {
            let depth = self.vector_count().min(super::MAX_LIMIT);
            let hits = match &plan.anchors {
                Some(anchors) => self.ordered_anchor_hits(
                    &plan.ids,
                    anchors,
                    scores.query,
                    depth,
                    scores.metric,
                )?,
                None => self.ordered_device_hits(context, scores.query, opts, depth)?,
            };
            scores.keep_emitted(hits);
        }
        Ok(())
    }

    /// Restores the default NEAR search's primary and component scores for
    /// admitted rows, after the chunk's vector and payload guards are gone.
    fn apply_native_scan_scores(
        &self,
        rows: &mut [SearchResult],
        scoring: &mut NativeScanScores<'_>,
    ) {
        let mut candidates: Vec<_> = rows
            .iter()
            .map(|r| crate::scored_result::ScoredResult::new(r.point.id, r.score))
            .collect();
        if let Some(source) = scoring.index {
            self.storage
                .index
                .score_admitted_rows(scoring.query, &mut candidates, source);
        }
        if scoring.pq {
            if let Some(rescored) = self.pq_scores(scoring.query, scoring.metric, &candidates) {
                candidates = rescored;
            }
        }
        for (row, score) in rows.iter_mut().zip(candidates) {
            if scoring
                .anchors
                .is_some_and(|anchors| !anchors.in_native_bitmap(row.point.id))
            {
                continue;
            }
            #[cfg(any(feature = "gpu", test))]
            if let Some(emitted) = scoring.emitted_score(row.point.id) {
                row.score = emitted;
                continue;
            }
            row.score = if scoring.pending.holds(row.point.id) {
                self.storage.index.score(scoring.query, &row.point.vector)
            } else {
                score.score
            };
        }
        super::super::tag_vector_component_scores(rows);
    }

    /// The ids an ordered scan of `stmt` reads and how it scores them: a
    /// `NEAR` reads only the rows storing a vector, the exact scan's shapes
    /// score by their first leaf. Its candidates are those the WHERE's
    /// AND-required graph predicates anchor, else those a secondary index
    /// holds for a WHERE made of metadata predicates only, else every row.
    fn ordered_scan_plan<const SIMILARITY_MAX: usize>(
        &self,
        stmt: &SelectStatement,
        params: &HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
        limit: usize,
    ) -> Result<ScanPlan> {
        let near = extracted
            .vector_search
            .clone()
            .filter(|_| !extracted.similarity_scan);
        let scope = if near.is_some() {
            RowScope::Embedded
        } else {
            RowScope::All
        };
        let leaf = match stmt.where_clause.as_ref() {
            Some(cond) => Self::ranking_leaf(cond, params)?,
            None => None,
        };
        let candidates = self.ordered_scan_candidates(stmt, params, extracted, graph_cache)?;
        let universe = scope.universe(self.row_universe());
        let ids =
            self.ordered_scan_ids::<SIMILARITY_MAX>(extracted, universe, candidates, limit)?;
        let anchors = match extracted.ordered_index_scores {
            Some(OrderedIndexScores::Anchors) => {
                self.ordered_anchors(ids.len(), extracted.filter_condition.as_ref())
            }
            _ => None,
        };
        Ok(ScanPlan {
            anchors,
            ids,
            universe,
            leaf,
            near,
        })
    }

    /// Reads an existing prefilter's ascending ids directly, or obtains the
    /// candidates of the exact or unrestricted scan.
    fn ordered_scan_ids<const SIMILARITY_MAX: usize>(
        &self,
        extracted: &ExtractedComponents,
        universe: RowUniverse,
        candidates: Option<HashSet<u64>>,
        limit: usize,
    ) -> Result<Vec<u64>> {
        if let Some(bitmap) = extracted
            .ordered_prefilter
            .as_ref()
            .filter(|p| p.covers_scan)
            .and_then(|p| p.bitmap.as_ref())
        {
            return Ok(bitmap.iter().map(u64::from).collect());
        }
        if extracted.similarity_scan {
            return self.similarity_scan_ids::<SIMILARITY_MAX>(candidates.as_ref(), limit);
        }
        let mut ids =
            candidates.map_or_else(|| self.ids_of(universe), |ids| ids.into_iter().collect());
        ids.sort_unstable();
        Ok(ids)
    }

    /// The ids every row an ordered scan admits is among, when fewer than
    /// every row: see [`Self::ordered_scan_plan`].
    fn ordered_scan_candidates(
        &self,
        stmt: &SelectStatement,
        params: &HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Option<HashSet<u64>>> {
        let Some(cond) = stmt.where_clause.as_ref() else {
            return Ok(None);
        };
        if !extracted.graph_match_predicates.is_empty() {
            return self.compute_required_anchor_ids(cond, params, &stmt.from_alias, graph_cache);
        }
        let metadata_only =
            extracted.vector_search.is_none() && extracted.similarity_conditions.is_empty();
        Ok(metadata_only
            .then(|| Self::extract_metadata_filter(cond))
            .flatten()
            .and_then(|metadata| self.ordered_metadata_candidates(metadata)))
    }

    /// A superset of matching ids when an index can narrow the metadata filter.
    /// `scan_candidate` checks each candidate against the full WHERE.
    fn ordered_metadata_candidates(
        &self,
        metadata: crate::velesql::Condition,
    ) -> Option<HashSet<u64>> {
        use crate::velesql::{condition_tree::any_subtree, CompareOp, Comparison, Condition};

        let has_range = any_subtree(&metadata, &|leaf| {
            matches!(
                leaf,
                Condition::Between(_)
                    | Condition::Comparison(Comparison {
                        operator: CompareOp::Gt | CompareOp::Gte | CompareOp::Lt | CompareOp::Lte,
                        ..
                    })
            )
        });
        // Secondary indexes read a literal top-level key (`backfill_single_payload`).
        // A WHERE with that dotted spelling follows a nested path.
        let nested = Self::index_reads_dotted_field(&metadata);
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata));
        if nested {
            return filter.condition.point_ids().map(HashSet::from_iter);
        }
        if has_range {
            if let Some(bitmap) = self.ordered_range_bitmap(&filter) {
                return Some(bitmap.iter().map(u64::from).collect());
            }
        }
        self.try_index_accelerated_ids(&filter)
            .map(HashSet::from_iter)
    }

    pub(super) fn index_reads_dotted_field(metadata: &crate::velesql::Condition) -> bool {
        use crate::velesql::{condition_tree::any_subtree, Condition};
        any_subtree(metadata, &|leaf| match leaf {
            Condition::Comparison(c) => c.column.contains('.'),
            Condition::Between(c) => c.column.contains('.'),
            Condition::In(c) => c.column.contains('.'),
            _ => false,
        })
    }

    /// Every row id of `universe`, read under the storage guards.
    fn ids_of(&self, universe: RowUniverse) -> Vec<u64> {
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        universe.ids(&vector_storage, &payload_storage)
    }

    /// The rows of `chunk` the WHERE of `stmt` admits, hydrated and scored
    /// ([`Self::scan_candidate`]), under one hold of the storage guards.
    // The two guards are lent to the evaluator for the whole chunk, so a
    // nested MATCH reuses them instead of re-acquiring them; vector_storage
    // (rank 2) before payload_storage (rank 3), after the metric (rank 1).
    #[expect(clippy::significant_drop_tightening)]
    fn scan_chunk(
        &self,
        chunk: &[u64],
        context: &QueryFinalizationContext<'_>,
        plan: &ScanPlan,
        now_secs: u64,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        let condition = context.stmt.where_clause.as_ref();
        // NEAR narrows the outer rows, but a MATCH may reach any stored graph node.
        let (metric, graph_rows) = {
            let config = self.storage.config.read();
            (config.metric, RowUniverse::of(&config))
        };
        let vector_storage = self.storage.vector_storage.read();
        let payload_storage = self.storage.payload_storage.read();
        let guards = super::match_exec::MatchStorageGuards::single(
            metric,
            graph_rows,
            &vector_storage,
            &payload_storage,
        );
        let scan = ScanCtx {
            vector_storage: &vector_storage,
            payload_storage: &payload_storage,
            universe: plan.universe,
            metric,
            now_secs,
            score: plan.score(),
            from_aliases: &context.stmt.from_alias,
        };
        let mut rows = Vec::new();
        for &id in chunk {
            if let Some((row, _)) =
                self.scan_candidate(id, condition, context.params, &scan, &guards, graph_cache)?
            {
                rows.push(row);
            }
        }
        Ok(rows)
    }
}

#[cfg(test)]
#[path = "ordered_scan_tests.rs"]
mod tests;

#[cfg(any(feature = "gpu", test))]
#[path = "ordered_scan_device.rs"]
mod device;

#[cfg(test)]
#[path = "ordered_scan_device_tests.rs"]
mod device_tests;
