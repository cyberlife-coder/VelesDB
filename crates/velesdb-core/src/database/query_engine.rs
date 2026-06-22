//! Query execution: `execute_query`, `explain_query`, `explain_analyze_query`, plan caching, and DML dispatch.

use crate::velesql::{
    ActualStats, AdminStatement, DdlStatement, DmlStatement, ExplainOutput, IntrospectionStatement,
    Query, TrainStatement,
};
use crate::{Error, Result, SearchResult};

use super::Database;

/// Statement type classification for dispatch routing.
enum StatementType<'a> {
    Admin(&'a AdminStatement),
    Introspection(&'a IntrospectionStatement),
    Ddl(&'a DdlStatement),
    Train(&'a TrainStatement),
    Dml(&'a DmlStatement),
    Match,
    Select,
}

/// Classifies a query into its statement type for routing.
fn classify_statement(query: &Query) -> StatementType<'_> {
    if let Some(admin) = query.admin.as_ref() {
        return StatementType::Admin(admin);
    }
    if let Some(intro) = query.introspection.as_ref() {
        return StatementType::Introspection(intro);
    }
    if let Some(ddl) = query.ddl.as_ref() {
        return StatementType::Ddl(ddl);
    }
    if let Some(train) = query.train.as_ref() {
        return StatementType::Train(train);
    }
    if let Some(dml) = query.dml.as_ref() {
        return StatementType::Dml(dml);
    }
    if query.is_match_query() {
        return StatementType::Match;
    }
    StatementType::Select
}

impl Database {
    /// Produces a canonical JSON string for a `serde_json::Value`.
    ///
    /// Recursively sorts the keys of every JSON object so that two values
    /// representing the same logical structure always produce identical bytes,
    /// regardless of the `HashMap` iteration order used during serialization.
    ///
    /// This is required because `FusionConfig::params` and
    /// `TrainStatement::params` are `HashMap`-backed; `serde_json` serialises
    /// them in hash-order, which is non-deterministic across invocations.
    fn canonical_json(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                // Without the `preserve_order` feature flag, `serde_json::Map` is already
                // backed by `BTreeMap` and therefore already sorted. This explicit sort
                // step is kept as defense-in-depth: if `preserve_order` is ever enabled
                // in `Cargo.toml` (which switches the backing store to `IndexMap` and
                // preserves insertion order), the canonical key ordering is still upheld
                // without any change to this function.
                let sorted: serde_json::Map<String, serde_json::Value> = map
                    .into_iter()
                    .map(|(k, v)| (k, Self::canonical_json(v)))
                    .collect::<std::collections::BTreeMap<_, _>>()
                    .into_iter()
                    .collect();
                serde_json::Value::Object(sorted)
            }
            serde_json::Value::Array(arr) => {
                serde_json::Value::Array(arr.into_iter().map(Self::canonical_json).collect())
            }
            other => other,
        }
    }

    /// Builds a deterministic cache key for a query (CACHE-02).
    ///
    /// Serialises the query to canonical JSON (object keys sorted recursively),
    /// reads the current `schema_version`, and gathers per-collection
    /// `write_generation` counters (sorted by collection name) to form a
    /// `PlanKey`.
    ///
    /// # Why canonical JSON instead of `Debug`
    ///
    /// `format!("{query:?}")` is non-deterministic when the `Query` AST
    /// contains `HashMap`-backed fields (`FusionConfig::params`,
    /// `TrainStatement::params`) because `HashMap` iteration order is not
    /// guaranteed across invocations. Canonical JSON with sorted object keys
    /// is stable and produces the same byte sequence for logically identical
    /// queries.
    #[must_use]
    pub fn build_plan_key(&self, query: &crate::velesql::Query) -> crate::cache::PlanKey {
        use std::hash::{BuildHasher, Hasher};

        // Serialise via serde_json, then canonicalise (sort object keys) before hashing.
        // Fallback to Debug representation if serialization fails (should never happen in
        // practice since all Query fields are Serialize, but erring on the side of liveness).
        let query_text = serde_json::to_value(query)
            .map(Self::canonical_json)
            .and_then(|v| serde_json::to_string(&v))
            .unwrap_or_else(|_| format!("{query:?}"));

        let mut hasher = rustc_hash::FxBuildHasher.build_hasher();
        hasher.write(query_text.as_bytes());
        let query_hash = hasher.finish();

        let schema_version = self.schema_version();
        let collection_names = Self::referenced_collection_names(query);

        // Build generations vector in sorted collection order.
        let collection_generations: smallvec::SmallVec<[u64; 4]> = collection_names
            .iter()
            .map(|name| self.collection_write_generation(name).unwrap_or(0))
            .collect();

        // Issue #608: parallel vector of analyze generations so that running
        // ANALYZE alone (no data mutation) still flips the cache key and
        // rebuilds plans with the fresh calibrated cost estimates.
        let analyze_generations: smallvec::SmallVec<[u64; 4]> = collection_names
            .iter()
            .map(|name| self.collection_analyze_generation(name).unwrap_or(0))
            .collect();

        crate::cache::PlanKey {
            // Issue #902: store the canonical text so PlanKey equality is
            // collision-safe. query_hash stays a Hash accelerator only.
            query_text: query_text.into(),
            query_hash,
            schema_version,
            collection_generations,
            analyze_generations,
        }
    }

    /// Returns the query plan for a query, with cache status populated (CACHE-02).
    ///
    /// If the plan is cached, returns it with `cache_hit: Some(true)` and
    /// `plan_reuse_count` set. Otherwise generates a fresh plan with
    /// `cache_hit: Some(false)`.
    ///
    /// # Design decision: `explain_query` does not populate the cache
    ///
    /// `explain_query` intentionally does **not** insert a new plan into the
    /// compiled plan cache. EXPLAIN is a diagnostic operation; allowing it to
    /// influence cache state would make cache metrics (hit/miss ratios,
    /// `plan_reuse_count`) unreliable because EXPLAIN calls would be
    /// indistinguishable from real execution hits. Only `execute_query` is
    /// authorised to write to the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid.
    pub fn explain_query(
        &self,
        query: &crate::velesql::Query,
    ) -> Result<crate::velesql::QueryPlan> {
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;

        let plan_key = self.build_plan_key(query);

        if let Some(cached) = self.compiled_plan_cache.get(&plan_key) {
            let mut plan = cached.plan.clone();
            plan.cache_hit = Some(true);
            plan.plan_reuse_count = Some(
                cached
                    .reuse_count
                    .load(std::sync::atomic::Ordering::Relaxed),
            );
            return Ok(plan);
        }

        let mut plan = self.build_plan_with_stats(query);
        plan.cache_hit = Some(false);
        plan.plan_reuse_count = Some(0);
        Ok(plan)
    }

    /// Builds a query plan, resolving calibrated collection statistics AND
    /// the registered secondary index set from the registry when available
    /// (#471 — EXPLAIN real costs, #607 — `IndexLookup` wiring).
    ///
    /// The returned plan's `estimated_cost_ms` and `filter_strategy` are
    /// calibrated via `CostEstimator` when stats exist for the query's
    /// primary collection. Falls back to heuristics otherwise. The
    /// `indexed_fields` argument is populated from
    /// `Database::indexed_fields_for` so that `IndexLookup` nodes appear
    /// in the EXPLAIN tree for WHERE clauses targeting indexed columns.
    fn build_plan_with_stats(&self, query: &crate::velesql::Query) -> crate::velesql::QueryPlan {
        let primary = &query.select.from;
        let core_stats = self.get_collection_stats(primary).ok().flatten();
        let indexed = self.indexed_fields_for(primary);
        crate::velesql::QueryPlan::from_query_with_stats(query, &indexed, core_stats.as_ref())
    }

    /// Executes a query with instrumentation and returns both plan and actual stats.
    ///
    /// Unlike `explain_query` (plan only) and `execute_query` (results only),
    /// this method returns the full [`ExplainOutput`] with measured statistics.
    /// The normal `execute_query` path is untouched — zero overhead on
    /// non-ANALYZE queries.
    ///
    /// # Errors
    ///
    /// Returns an error if the query is invalid or execution fails.
    pub fn explain_analyze_query(
        &self,
        query: &Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<ExplainOutput> {
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;

        let plan = self.explain_query(query)?;
        let start = std::time::Instant::now();
        let (results, nodes_visited, edges_traversed) =
            self.execute_query_counted(query, params)?;
        let elapsed = start.elapsed();

        let actual_rows = results.len() as u64;
        let actual_time_ms = elapsed.as_secs_f64() * 1000.0;

        let stats = ActualStats {
            actual_rows,
            actual_time_ms,
            loops: 1,
            nodes_visited,
            edges_traversed,
        };

        let node_stats =
            crate::velesql::build_leaf_node_stats(&plan.root, actual_rows, actual_time_ms);
        Ok(ExplainOutput::with_stats(plan, stats, node_stats))
    }

    /// Executes a `VelesQL` query with database-level JOIN resolution.
    ///
    /// This method resolves JOIN target collections from the database registry
    /// and executes JOIN runtime in sequence. Query plans are cached and
    /// reused for identical queries against unchanged collections (CACHE-02).
    ///
    /// # Errors
    ///
    /// Returns an error if the base collection or any JOIN collection is missing.
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;

        if let Some(results) = self.dispatch_non_select(query, params)? {
            return Ok(results);
        }

        // Build plan key and check cache WITHOUT recording hit/miss metrics (CACHE-02).
        //
        // `contains()` is used instead of `get().is_some()` so that this
        // existence check does not increment the hit/miss counters or
        // `reuse_count`. Only `explain_query` (which surfaces these values to
        // callers) should call `get()`.
        let pre_exec_key = self.build_plan_key(query);
        let is_cached = self.compiled_plan_cache.contains(&pre_exec_key);

        let results = self.execute_select_query(query, params)?;

        // Populate cache on miss (CACHE-02).
        //
        // C-1 TOCTOU fix: rebuild the plan key AFTER execution. Between the
        // pre-execution `contains()` check and here, a concurrent writer may
        // have bumped a collection's `write_generation` (e.g. via `upsert` on
        // another thread). Rebuilding the key captures the post-execution
        // state, so the cached plan is associated with the generation that was
        // live when the plan was actually compiled — not a potentially stale
        // pre-execution snapshot.
        if !is_cached {
            self.populate_plan_cache(query);
        }

        Ok(results)
    }

    /// Classifies and dispatches non-SELECT statement types.
    ///
    /// Returns `Ok(Some(results))` if handled, `Ok(None)` for SELECT queries.
    fn dispatch_non_select(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Option<Vec<SearchResult>>> {
        // Classify the statement type (at most one is Some).
        let stmt_type = classify_statement(query);
        match stmt_type {
            StatementType::Admin(admin) => Ok(Some(self.execute_admin(admin)?)),
            StatementType::Introspection(intro) => Ok(Some(self.execute_introspection(intro)?)),
            StatementType::Ddl(ddl) => Ok(Some(self.execute_ddl(ddl)?)),
            StatementType::Train(train) => Ok(Some(self.execute_train(train)?)),
            StatementType::Dml(dml) => Ok(Some(self.execute_dml(dml, params)?)),
            StatementType::Match => Ok(Some(self.execute_match_routed(query, params)?.0)),
            StatementType::Select => Ok(None),
        }
    }

    /// Resolves the target collection for a MATCH query.
    ///
    /// Resolution order: `SELECT ... FROM <collection> WHERE MATCH ...`, then a
    /// `"_collection"` key in `params` (programmatic API), else a guidance error.
    fn resolve_match_collection(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<crate::collection::Collection> {
        let collection_name = if !query.select.from.is_empty() {
            query.select.from.clone()
        } else if let Some(serde_json::Value::String(name)) = params.get("_collection") {
            name.clone()
        } else {
            return Err(Error::Query(
                "MATCH query requires a target collection. Either use \
                 SELECT ... FROM <collection> WHERE MATCH ..., or pass \
                 {\"_collection\": \"name\"} in params."
                    .to_string(),
            ));
        };
        self.resolve_collection(&collection_name)
    }

    /// Routes a MATCH query to its target collection and applies cross-collection
    /// enrichment, returning results plus the graph-traversal counters
    /// `(nodes_visited, edges_traversed)` measured during execution (for
    /// EXPLAIN ANALYZE; the plain execution path discards them).
    fn execute_match_routed(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(Vec<SearchResult>, u64, u64)> {
        let coll = self.resolve_match_collection(query, params)?;
        let (mut results, nodes_visited, edges_traversed) =
            coll.execute_query_counted(query, params)?;
        // Cross-collection enrichment: if any node pattern has a @collection
        // annotation, look up payloads from those collections and merge them
        // into the projected fields.
        if let Some(mc) = &query.match_clause {
            self.enrich_match_results_cross_collection(mc, &mut results);
        }
        Ok((results, nodes_visited, edges_traversed))
    }

    /// Executes a query and returns graph-traversal counters for EXPLAIN ANALYZE.
    ///
    /// MATCH queries report real `(nodes_visited, edges_traversed)`; every other
    /// statement type reports `(_, 0, 0)` (no graph traversal occurred).
    fn execute_query_counted(
        &self,
        query: &Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(Vec<SearchResult>, u64, u64)> {
        if query.is_match_query() {
            return self.execute_match_routed(query, params);
        }
        Ok((self.execute_query(query, params)?, 0, 0))
    }

    /// Executes the SELECT portion of a query, resolving JOINs if present.
    fn execute_select_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        // EPIC-040 US-006: For compound queries, strip LIMIT from each operand so
        // the set operation sees the full result sets.  The final LIMIT is applied
        // once on the merged output (SQL-standard behaviour).
        // Use MAX_LIMIT (not None) to avoid the default-10 cap downstream.
        const COMPOUND_LIMIT: usize = 100_000;
        let compound_limit = Some(COMPOUND_LIMIT as u64); // 100_000 fits u64 exactly.
        let left_results = if query.compound.is_some() {
            let mut left_query = query.clone();
            left_query.select.limit = compound_limit;
            self.execute_single_select(&left_query, params)?
        } else {
            return self.execute_single_select(query, params);
        };

        // compound is guaranteed Some here (non-compound returns above).
        if let Some(ref compound) = query.compound {
            let mut accumulated = left_results;
            for (operator, right_select) in &compound.operations {
                let mut right_query = crate::velesql::Query::new_select(right_select.clone());
                right_query.select.limit = compound_limit;
                let right_results = self.execute_single_select(&right_query, params)?;
                accumulated = crate::collection::search::query::set_operations::apply_set_operation(
                    accumulated,
                    right_results,
                    *operator,
                    // Intermediate ops keep the server-side ceiling: truncating to the
                    // user LIMIT here would drop rows a later chained set op still needs.
                    COMPOUND_LIMIT,
                );
            }
            // SQL-standard: LIMIT from the left (outer) SELECT applies to the final result.
            if let Some(limit) = query.select.limit {
                accumulated.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            }
            return Ok(accumulated);
        }

        Ok(left_results)
    }

    /// Collects sorted, deduplicated collection names referenced by a query,
    /// including all compound operands (UNION, INTERSECT, EXCEPT).
    ///
    /// RF-DEDUP: Shared by `build_plan_key` and `populate_plan_cache`, which
    /// both need the same sorted collection-name list from the query AST.
    fn referenced_collection_names(query: &crate::velesql::Query) -> Vec<String> {
        let mut names = vec![query.select.from.clone()];
        for join in &query.select.joins {
            names.push(join.table.clone());
        }
        if let Some(ref compound) = query.compound {
            for (_, right_select) in &compound.operations {
                names.push(right_select.from.clone());
                for join in &right_select.joins {
                    names.push(join.table.clone());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// Resolves a collection by name from all typed registries.
    ///
    /// Priority: vector collections first, then graph, then metadata.
    /// Returns the inner `Collection` for query execution.
    pub(super) fn resolve_collection(&self, name: &str) -> Result<crate::collection::Collection> {
        if let Some(vc) = self.get_vector_collection(name) {
            return Ok(vc.inner);
        }
        if let Some(gc) = self.get_graph_collection(name) {
            return Ok(gc.inner);
        }
        if let Some(mc) = self.get_metadata_collection(name) {
            return Ok(mc.inner);
        }
        Err(Error::CollectionNotFound(name.to_string()))
    }

    /// Resolves a collection that supports write operations (INSERT/UPDATE/TRAIN).
    ///
    /// Checks vector, graph, and metadata collections. Metadata-only collections
    /// support INSERT/UPDATE for metadata fields (no vectors).
    pub(super) fn resolve_writable_collection(
        &self,
        name: &str,
    ) -> Result<crate::collection::Collection> {
        if let Some(vc) = self.get_vector_collection(name) {
            return Ok(vc.inner);
        }
        if let Some(gc) = self.get_graph_collection(name) {
            return Ok(gc.inner);
        }
        if let Some(mc) = self.get_metadata_collection(name) {
            return Ok(mc.inner);
        }
        Err(Error::CollectionNotFound(name.to_string()))
    }

    /// Executes a single SELECT (no compound), resolving JOINs if present.
    ///
    /// Orchestrates filter pushdown and join strategy selection:
    /// 1. Analyze WHERE for pushdown-eligible conditions
    /// 2. Strip pushed conditions from base query
    /// 3. For each JOIN: lookup, filtered, or full `ColumnStore` path
    /// 4. Apply post-join filters (cross-source predicates)
    fn execute_single_select(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let base_collection = self.resolve_collection(&query.select.from)?;

        let mut single_query = query.clone();
        single_query.compound = None;

        if single_query.select.joins.is_empty() {
            return base_collection.execute_query(&single_query, params);
        }

        let analysis = Self::prepare_join_pushdown(&mut single_query, params)?;
        let pushed = analysis.column_store_filters.clone();

        let row_budget = Self::join_row_budget(&query.select, &analysis);

        let mut results = base_collection.execute_query(&single_query, params)?;
        for join in &query.select.joins {
            results = self.execute_single_join(&results, join, &pushed, row_budget)?;
        }

        // Apply post-join filters: cross-source predicates that reference
        // columns from both the base collection and joined ColumnStore tables.
        if !analysis.post_join_filters.is_empty() {
            results = Self::apply_post_join_filters(
                &base_collection,
                results,
                &analysis.post_join_filters,
                params,
                &query.select.from_alias,
            )?;
        }

        Ok(results)
    }

    /// Resolves WHERE parameters, runs pushdown analysis, and strips pushed
    /// conditions from the base query (JOIN path of `execute_single_select`).
    ///
    /// Parameter placeholders are resolved before the analysis so pushed-down
    /// filters never silently convert them to NULL at the `ColumnStore` layer
    /// (the collection pipeline resolves its own copy independently).
    fn prepare_join_pushdown(
        single_query: &mut crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<crate::collection::search::query::pushdown::PushdownAnalysis> {
        if let Some(cond) = single_query.select.where_clause.take() {
            single_query.select.where_clause = Some(
                crate::collection::Collection::resolve_condition_params(&cond, params)?,
            );
        }

        let analysis = Self::analyze_join_pushdown_for_select(&single_query.select);

        let resolved_where = single_query.select.where_clause.clone();
        single_query.select.joins.clear();
        if !analysis.column_store_filters.is_empty() {
            single_query.select.where_clause = Self::strip_pushed_conditions(
                resolved_where.as_ref(),
                &analysis.column_store_filters,
            );
        }
        Ok(analysis)
    }

    /// Computes the bound on joined rows to materialize.
    ///
    /// When the query has an explicit LIMIT, no post-join filters, and no
    /// ORDER BY (which could reorder past the window), the bound is the
    /// effective `LIMIT + OFFSET`. GROUP BY / HAVING / DISTINCT also disqualify
    /// the bounded shape: SQL LIMIT bounds output *groups/rows*, not input rows,
    /// so truncating joined input to `LIMIT` would drop rows that belong to
    /// groups still inside the window. Otherwise downstream stages may reorder or
    /// drop rows, so we fall back to the conservative server-side ceiling
    /// [`JOIN_ROW_CEILING`] — still bounding OOM without affecting correctness.
    pub(super) fn join_row_budget(
        select: &crate::velesql::SelectStatement,
        analysis: &crate::collection::search::query::pushdown::PushdownAnalysis,
    ) -> usize {
        use crate::collection::search::query::JOIN_ROW_CEILING;
        use crate::velesql::DistinctMode;
        let bounded_shape = analysis.post_join_filters.is_empty()
            && select.order_by.is_none()
            && select.group_by.is_none()
            && select.having.is_none()
            && select.distinct == DistinctMode::None;
        match select.limit {
            Some(limit) if bounded_shape => {
                let limit = usize::try_from(limit).unwrap_or(JOIN_ROW_CEILING);
                let offset = select
                    .offset
                    .map_or(0, |o| usize::try_from(o).unwrap_or(JOIN_ROW_CEILING));
                limit.saturating_add(offset).min(JOIN_ROW_CEILING)
            }
            _ => JOIN_ROW_CEILING,
        }
    }

    // NOTE: analyze_join_pushdown_for_select, apply_post_join_filters
    // moved to join_pushdown.rs (NLOC/file reduction)

    /// Inserts a compiled plan into the cache after a cache miss (CACHE-02).
    fn populate_plan_cache(&self, query: &crate::velesql::Query) {
        let compiled = std::sync::Arc::new(crate::cache::CompiledPlan {
            plan: self.build_plan_with_stats(query),
            referenced_collections: Self::referenced_collection_names(query),
            compiled_at: std::time::Instant::now(),
            reuse_count: std::sync::atomic::AtomicU64::new(0),
        });
        // Rebuild key after execution to reflect current write_generation (C-1).
        let post_exec_key = self.build_plan_key(query);
        self.compiled_plan_cache.insert(post_exec_key, compiled);
    }

    /// Dispatches a DML statement (INSERT, UPSERT, UPDATE, DELETE, or edge mutations).
    pub(super) fn execute_dml(
        &self,
        dml: &crate::velesql::DmlStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        match dml {
            crate::velesql::DmlStatement::Insert(stmt)
            | crate::velesql::DmlStatement::Upsert(stmt) => self.execute_insert(stmt, params),
            crate::velesql::DmlStatement::Update(stmt) => self.execute_update(stmt, params),
            crate::velesql::DmlStatement::InsertEdge(stmt) => self.execute_insert_edge(stmt),
            crate::velesql::DmlStatement::Delete(stmt) => self.execute_delete(stmt),
            crate::velesql::DmlStatement::DeleteEdge(stmt) => self.execute_delete_edge(stmt),
            crate::velesql::DmlStatement::SelectEdges(stmt) => self.execute_select_edges(stmt),
            crate::velesql::DmlStatement::InsertNode(stmt) => self.execute_insert_node(stmt),
        }
    }
}
