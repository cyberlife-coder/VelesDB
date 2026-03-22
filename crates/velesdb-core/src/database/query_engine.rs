//! Query execution: `execute_query`, `explain_query`, plan caching, and DML dispatch.

use crate::{Error, Result, SearchResult};

use super::Database;

#[allow(deprecated)] // Uses legacy Collection internally for query routing.
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

        crate::cache::PlanKey {
            query_hash,
            schema_version,
            collection_generations,
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

        let mut plan = crate::velesql::QueryPlan::from_select(&query.select);
        plan.cache_hit = Some(false);
        plan.plan_reuse_count = Some(0);
        Ok(plan)
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
    #[allow(clippy::too_many_lines)]
    pub fn execute_query(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        crate::velesql::QueryValidator::validate(query).map_err(|e| Error::Query(e.to_string()))?;

        if let Some(train) = query.train.as_ref() {
            return self.execute_train(train);
        }

        if let Some(dml) = query.dml.as_ref() {
            return self.execute_dml(dml, params);
        }

        if query.is_match_query() {
            return Err(Error::Query(
                "Database::execute_query does not support top-level MATCH queries. Use Collection::execute_query or pass the collection name."
                    .to_string(),
            ));
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
        let compound_limit = Some(100_000_u64);
        let left_results = if query.compound.is_some() {
            let mut left_query = query.clone();
            left_query.select.limit = compound_limit;
            self.execute_single_select(&left_query, params)?
        } else {
            return self.execute_single_select(query, params);
        };

        // compound is guaranteed Some here (non-compound returns above).
        if let Some(ref compound) = query.compound {
            let mut right_query = crate::velesql::Query::new_select(*compound.right.clone());
            right_query.select.limit = compound_limit;
            let right_results = self.execute_single_select(&right_query, params)?;
            let mut merged = crate::collection::search::query::set_operations::apply_set_operation(
                left_results,
                right_results,
                compound.operator,
            );
            // SQL-standard: LIMIT from the left (outer) SELECT applies to the final result.
            if let Some(limit) = query.select.limit {
                merged.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
            }
            return Ok(merged);
        }

        Ok(left_results)
    }

    /// Collects sorted, deduplicated collection names referenced by a query.
    ///
    /// RF-DEDUP: Shared by `build_plan_key` and `populate_plan_cache`, which
    /// both need the same sorted collection-name list from the query AST.
    fn referenced_collection_names(query: &crate::velesql::Query) -> Vec<String> {
        let mut names = vec![query.select.from.clone()];
        for join in &query.select.joins {
            names.push(join.table.clone());
        }
        names.sort();
        names.dedup();
        names
    }

    /// Resolves a collection by name from all registries (legacy, vector, metadata).
    ///
    /// Priority: legacy collections registry first (contains live instances for both
    /// `create_collection` and `create_vector_collection` via shared inner `Arc<>`).
    /// Falls back to vector collections, then metadata collections.
    ///
    /// RF-DEDUP: Shared by `execute_single_select` (reads are valid on all collection
    /// types, including metadata).
    #[allow(deprecated)]
    pub(super) fn resolve_collection(&self, name: &str) -> Result<crate::collection::Collection> {
        self.get_collection(name)
            .or_else(|| self.get_vector_collection(name).map(|vc| vc.inner))
            .or_else(|| self.get_metadata_collection(name).map(|mc| mc.inner))
            .ok_or_else(|| Error::CollectionNotFound(name.to_string()))
    }

    /// Resolves a collection that supports write operations (INSERT/UPDATE/TRAIN).
    ///
    /// Only checks legacy and vector collections — metadata-only collections do not
    /// support INSERT with vectors, UPDATE with vectors, or TRAIN QUANTIZER, so
    /// resolving them here would produce misleading errors deeper in the pipeline.
    #[allow(deprecated)]
    pub(super) fn resolve_writable_collection(
        &self,
        name: &str,
    ) -> Result<crate::collection::Collection> {
        self.get_collection(name)
            .or_else(|| self.get_vector_collection(name).map(|vc| vc.inner))
            .ok_or_else(|| Error::CollectionNotFound(name.to_string()))
    }

    /// Executes a single SELECT (no compound), resolving JOINs if present.
    fn execute_single_select(
        &self,
        query: &crate::velesql::Query,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let base_collection = self.resolve_collection(&query.select.from)?;

        // Strip compound from the query before delegating to Collection::execute_query,
        // because compound handling is done by execute_select_query (our caller).
        // Without this, the set operation would be applied twice (once at Collection
        // level, once here) — causing e.g. UNION ALL to duplicate right-side results.
        let mut single_query = query.clone();
        single_query.compound = None;

        if single_query.select.joins.is_empty() {
            return base_collection.execute_query(&single_query, params);
        }

        single_query.select.joins.clear();

        let mut results = base_collection.execute_query(&single_query, params)?;
        for join in &query.select.joins {
            let join_collection = self.resolve_collection(&join.table)?;
            let column_store = Self::build_join_column_store(&join_collection)?;
            let joined = crate::collection::search::query::join::execute_join(
                &results,
                join,
                &column_store,
            )?;
            results = crate::collection::search::query::join::joined_to_search_results(joined);
        }
        Ok(results)
    }

    /// Inserts a compiled plan into the cache after a cache miss (CACHE-02).
    fn populate_plan_cache(&self, query: &crate::velesql::Query) {
        let compiled = std::sync::Arc::new(crate::cache::CompiledPlan {
            plan: crate::velesql::QueryPlan::from_select(&query.select),
            referenced_collections: Self::referenced_collection_names(query),
            compiled_at: std::time::Instant::now(),
            reuse_count: std::sync::atomic::AtomicU64::new(0),
        });
        // Rebuild key after execution to reflect current write_generation (C-1).
        let post_exec_key = self.build_plan_key(query);
        self.compiled_plan_cache.insert(post_exec_key, compiled);
    }

    /// Dispatches a DML statement (INSERT or UPDATE).
    pub(super) fn execute_dml(
        &self,
        dml: &crate::velesql::DmlStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        match dml {
            crate::velesql::DmlStatement::Insert(stmt) => self.execute_insert(stmt, params),
            crate::velesql::DmlStatement::Update(stmt) => self.execute_update(stmt, params),
        }
    }

    /// Executes an INSERT statement.
    #[allow(deprecated)]
    fn execute_insert(
        &self,
        stmt: &crate::velesql::InsertStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let collection = self.resolve_writable_collection(&stmt.table)?;

        let (id, vector, payload) = Self::resolve_insert_fields(stmt, params)?;
        let point_id =
            id.ok_or_else(|| Error::Query("INSERT requires integer 'id' column".to_string()))?;
        let point = Self::build_insert_point(&collection, point_id, vector, payload)?;

        let result = SearchResult::new(point.clone(), 0.0);
        collection.upsert(vec![point])?;
        Ok(vec![result])
    }

    /// Resolves column values from an INSERT statement into id, vector, and payload fields.
    #[allow(clippy::type_complexity)] // Reason: one-off tuple return for internal helper.
    fn resolve_insert_fields(
        stmt: &crate::velesql::InsertStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<(
        Option<u64>,
        Option<Vec<f32>>,
        serde_json::Map<String, serde_json::Value>,
    )> {
        let mut id: Option<u64> = None;
        let mut payload = serde_json::Map::new();
        let mut vector: Option<Vec<f32>> = None;

        for (column, value_expr) in stmt.columns.iter().zip(&stmt.values) {
            let resolved = Self::resolve_dml_value(value_expr, params)?;
            if column == "id" {
                id = Some(Self::json_to_u64_id(&resolved)?);
                continue;
            }
            if column == "vector" {
                vector = Some(Self::json_to_vector(&resolved)?);
                continue;
            }
            payload.insert(column.clone(), resolved);
        }

        Ok((id, vector, payload))
    }

    /// Builds a `Point` for an INSERT statement, validating vector presence.
    fn build_insert_point(
        collection: &crate::Collection,
        point_id: u64,
        vector: Option<Vec<f32>>,
        payload: serde_json::Map<String, serde_json::Value>,
    ) -> Result<crate::Point> {
        if collection.is_metadata_only() {
            if vector.is_some() {
                return Err(Error::Query(
                    "INSERT on metadata-only collection cannot set 'vector'".to_string(),
                ));
            }
            Ok(crate::Point::metadata_only(
                point_id,
                serde_json::Value::Object(payload),
            ))
        } else {
            let vec_value = vector.ok_or_else(|| {
                Error::Query("INSERT on vector collection requires 'vector' column".to_string())
            })?;
            Ok(crate::Point::new(
                point_id,
                vec_value,
                Some(serde_json::Value::Object(payload)),
            ))
        }
    }

    /// Executes an UPDATE statement.
    #[allow(deprecated)]
    fn execute_update(
        &self,
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<SearchResult>> {
        let collection = self.resolve_writable_collection(&stmt.table)?;

        let assignments = Self::resolve_update_assignments(stmt, params)?;
        let filter = Self::build_update_filter(stmt.where_clause.as_ref())?;

        let all_ids = collection.all_ids();
        let rows = collection.get(&all_ids);
        let updated_points =
            Self::apply_update_assignments(&collection, rows, filter.as_ref(), &assignments)?;

        Self::upsert_and_collect(&collection, updated_points)
    }

    /// Resolves and validates UPDATE assignment values.
    fn resolve_update_assignments(
        stmt: &crate::velesql::UpdateStatement,
        params: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<Vec<(String, serde_json::Value)>> {
        let assignments = stmt
            .assignments
            .iter()
            .map(|a| Ok((a.column.clone(), Self::resolve_dml_value(&a.value, params)?)))
            .collect::<Result<Vec<_>>>()?;

        if assignments.iter().any(|(name, _)| name == "id") {
            return Err(Error::Query(
                "UPDATE cannot modify primary key column 'id'".to_string(),
            ));
        }
        Ok(assignments)
    }

    /// Upserts updated points and returns them as search results.
    #[allow(deprecated)]
    fn upsert_and_collect(
        collection: &crate::Collection,
        updated_points: Vec<crate::Point>,
    ) -> Result<Vec<SearchResult>> {
        if updated_points.is_empty() {
            return Ok(Vec::new());
        }
        let results = updated_points
            .iter()
            .map(|p| SearchResult::new(p.clone(), 0.0))
            .collect();
        collection.upsert(updated_points)?;
        Ok(results)
    }

    /// Applies field assignments to matching points, producing updated points.
    fn apply_update_assignments(
        collection: &crate::Collection,
        rows: Vec<Option<crate::Point>>,
        filter: Option<&crate::Filter>,
        assignments: &[(String, serde_json::Value)],
    ) -> Result<Vec<crate::Point>> {
        let mut updated_points = Vec::new();
        for point in rows.into_iter().flatten() {
            if !Self::matches_update_filter(&point, filter) {
                continue;
            }

            let mut payload_map = point
                .payload
                .as_ref()
                .and_then(serde_json::Value::as_object)
                .cloned()
                .unwrap_or_default();

            let mut updated_vector = point.vector.clone();

            for (field, value) in assignments {
                if field == "vector" {
                    if collection.is_metadata_only() {
                        return Err(Error::Query(
                            "UPDATE on metadata-only collection cannot set 'vector'".to_string(),
                        ));
                    }
                    updated_vector = Self::json_to_vector(value)?;
                } else {
                    payload_map.insert(field.clone(), value.clone());
                }
            }

            let updated = if collection.is_metadata_only() {
                crate::Point::metadata_only(point.id, serde_json::Value::Object(payload_map))
            } else {
                crate::Point::new(
                    point.id,
                    updated_vector,
                    Some(serde_json::Value::Object(payload_map)),
                )
            };
            updated_points.push(updated);
        }
        Ok(updated_points)
    }
}
