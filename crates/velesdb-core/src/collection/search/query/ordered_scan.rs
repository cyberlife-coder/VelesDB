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

use super::options::QueryFinalizationContext;
use super::row_universe::{RowScope, RowUniverse};
use super::similarity_scan::{RankingLeaf, RowScore, ScanCtx};
use super::ExtractedComponents;
use crate::collection::expiry::now_unix_secs;
use crate::collection::types::Collection;
use crate::error::Result;
use crate::point::SearchResult;
use crate::velesql::{LetBinding, Query, SelectOrderBy, SelectStatement};

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
    ids: Vec<u64>,
    universe: RowUniverse,
    leaf: Option<RankingLeaf>,
    near: Option<Vec<f32>>,
}

impl ScanPlan {
    fn score(&self) -> RowScore<'_> {
        match (&self.near, &self.leaf) {
            (Some(near), _) => RowScore::Near(near),
            (None, Some(leaf)) => RowScore::Leaf(leaf),
            (None, None) => RowScore::Unscored,
        }
    }
}

impl Collection {
    /// Runs the SELECT of `query` by the ordered scan, then its
    /// post-processing and guard-rails ([`Self::finalize_query_results`]).
    ///
    /// # Errors
    ///
    /// An unresolvable vector, an evaluation error (a nested MATCH), or a
    /// guard-rail violation.
    pub(super) fn execute_ordered_scan(
        &self,
        query: &Query,
        params: &HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        limit: usize,
        ctx: &crate::guardrails::QueryContext,
    ) -> Result<Vec<SearchResult>> {
        let stmt = &query.select;
        if !query.let_bindings.is_empty() {
            Self::validate_let_binding_support(extracted)?;
        }
        let mut results = self
            .ordered_scan_rows(stmt, params, extracted, &query.let_bindings, limit)
            .inspect_err(|e| self.runtime.guard_rails.circuit_breaker.record_error(e))?;
        self.finalize_query_results(
            &mut results,
            &QueryFinalizationContext {
                stmt,
                params,
                limit,
                extracted,
                ctx,
                let_bindings: &query.let_bindings,
            },
        )?;
        Ok(results)
    }

    /// The rows of the window of `stmt` — its leading `OFFSET + limit` rows
    /// by its `ORDER BY` among every row it admits — in no defined order.
    fn ordered_scan_rows(
        &self,
        stmt: &SelectStatement,
        params: &HashMap<String, serde_json::Value>,
        extracted: &ExtractedComponents,
        let_bindings: &[LetBinding],
        limit: usize,
    ) -> Result<Vec<SearchResult>> {
        let offset = stmt
            .offset
            .map_or(0, |offset| usize::try_from(offset).unwrap_or(usize::MAX));
        let mut window = LeadingRows {
            collection: self,
            order_by: stmt.order_by.as_deref().unwrap_or_default(),
            params,
            let_bindings,
            leading: offset.saturating_add(limit),
            rows: Vec::new(),
        };
        let mut graph_cache = super::where_eval::GraphMatchEvalCache::default();
        let plan = self.ordered_scan_plan::<{ Self::SIMILARITY_SCAN_MAX }>(
            stmt,
            params,
            extracted,
            &mut graph_cache,
            window.leading,
        )?;
        let now_secs = now_unix_secs();
        for chunk in plan.ids.chunks(CHUNK) {
            let rows = self.scan_chunk(chunk, stmt, params, &plan, now_secs, &mut graph_cache)?;
            window.extend(rows)?;
        }
        window.into_rows()
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
        let ids = if extracted.similarity_scan {
            self.similarity_scan_ids::<SIMILARITY_MAX>(candidates.as_ref(), limit)?
        } else {
            let mut ids = match candidates {
                Some(ids) => ids.into_iter().collect(),
                None => self.ids_of(universe),
            };
            ids.sort_unstable();
            ids
        };
        Ok(ScanPlan {
            ids,
            universe,
            leaf,
            near,
        })
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
        let nested = any_subtree(&metadata, &|leaf| match leaf {
            Condition::Comparison(c) => c.column.contains('.'),
            Condition::Between(c) => c.column.contains('.'),
            Condition::In(c) => c.column.contains('.'),
            _ => false,
        });
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata));
        if nested {
            return filter.condition.point_ids().map(HashSet::from_iter);
        }
        if has_range {
            if let Some(bitmap) = self.build_prefilter_bitmap(&filter) {
                return Some(bitmap.iter().map(u64::from).collect());
            }
        }
        self.try_index_accelerated_ids(&filter)
            .map(HashSet::from_iter)
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
        stmt: &SelectStatement,
        params: &HashMap<String, serde_json::Value>,
        plan: &ScanPlan,
        now_secs: u64,
        graph_cache: &mut super::where_eval::GraphMatchEvalCache,
    ) -> Result<Vec<SearchResult>> {
        let condition = stmt.where_clause.as_ref();
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
            from_aliases: &stmt.from_alias,
        };
        let mut rows = Vec::new();
        for &id in chunk {
            if let Some((row, _)) =
                self.scan_candidate(id, condition, params, &scan, &guards, graph_cache)?
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
