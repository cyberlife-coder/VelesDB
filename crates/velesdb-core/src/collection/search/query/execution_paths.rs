use super::{Collection, HashSet, QuerySearchOptions, Result, SearchResult, MAX_LIMIT};

impl Collection {
    pub(super) fn execute_indexed_metadata_query(
        &self,
        cond: &crate::velesql::Condition,
        execution_limit: usize,
    ) -> Option<Vec<SearchResult>> {
        // Try simple Eq lookup first (fastest path).
        if let Some((field_name, key)) = Self::extract_index_lookup_condition(cond) {
            let ids = self.secondary_index_lookup(&field_name, &key)?;
            tracing::debug!(
                field = %field_name,
                ids_count = ids.len(),
                limit = execution_limit,
                "indexed metadata query: Eq lookup"
            );
            // Skip index path when too many hits — sequential scan with early
            // exit is faster than hydrating thousands of index results.
            if ids.len() > execution_limit.saturating_mul(50).max(1000) {
                tracing::debug!("indexed metadata query: too many hits, falling through to scan");
                return None; // Fall through to scan
            }
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
            return Some(self.scan_ids_with_filter(&ids, &filter, execution_limit));
        }

        // For AND conditions, find the first Eq sub-condition that has an index,
        // use it to narrow the candidate set, then post-filter the rest.
        if let crate::velesql::Condition::And(ref _left, ref _right) = cond {
            // Flatten the AND tree into a list of leaf conditions.
            let mut leaves = Vec::new();
            Self::flatten_and_conditions(cond, &mut leaves);
            for sub in &leaves {
                if let Some((field_name, key)) = Self::extract_index_lookup_condition(sub) {
                    if let Some(ids) = self.secondary_index_lookup(&field_name, &key) {
                        let filter = crate::filter::Filter::new(crate::filter::Condition::from(
                            cond.clone(),
                        ));
                        return Some(self.scan_ids_with_filter(&ids, &filter, execution_limit));
                    }
                }
            }
        }

        None
    }

    /// Flattens a binary AND tree into a list of leaf conditions.
    fn flatten_and_conditions<'a>(
        cond: &'a crate::velesql::Condition,
        out: &mut Vec<&'a crate::velesql::Condition>,
    ) {
        match cond {
            crate::velesql::Condition::And(left, right) => {
                Self::flatten_and_conditions(left, out);
                Self::flatten_and_conditions(right, out);
            }
            crate::velesql::Condition::Group(inner) => {
                Self::flatten_and_conditions(inner, out);
            }
            other => out.push(other),
        }
    }

    /// Scans a set of candidate IDs and applies a filter, returning matching results.
    ///
    /// Uses score `1.0` for metadata-only matches (no vector similarity involved).
    fn scan_ids_with_filter(
        &self,
        ids: &[u64],
        filter: &crate::filter::Filter,
        execution_limit: usize,
    ) -> Vec<SearchResult> {
        let mut results = Vec::new();
        for point in self.get(ids).into_iter().flatten() {
            let payload = point.payload.clone().unwrap_or(serde_json::Value::Null);
            if filter.matches(&payload) {
                results.push(SearchResult::new(point, 1.0));
                if results.len() >= execution_limit {
                    break;
                }
            }
        }
        results
    }

    fn extract_index_lookup_condition(
        cond: &crate::velesql::Condition,
    ) -> Option<(String, crate::index::JsonValue)> {
        if let crate::velesql::Condition::Comparison(cmp) = cond {
            if cmp.operator == crate::velesql::CompareOp::Eq {
                return crate::index::JsonValue::from_ast_value(&cmp.value)
                    .map(|v| (cmp.column.clone(), v));
            }
        }
        None
    }

    /// Scans candidate IDs from a bitmap and applies the full filter.
    fn scan_candidate_ids_metadata(
        &self,
        candidate_ids: &[u64],
        filter: &crate::filter::Filter,
        limit: usize,
    ) -> Vec<SearchResult> {
        let mut results = Vec::new();
        for point in self.get(candidate_ids).into_iter().flatten() {
            let payload = point.payload.clone().unwrap_or(serde_json::Value::Null);
            if filter.matches(&payload) {
                results.push(SearchResult::new(point, 1.0));
                if results.len() >= limit {
                    break;
                }
            }
        }
        results
    }

    pub(crate) fn evaluate_graph_match_anchor_ids(
        &self,
        predicate: &crate::velesql::GraphMatchPredicate,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
    ) -> Result<HashSet<u64>> {
        let anchor_alias = Self::resolve_anchor_alias(predicate, from_aliases)?;
        let clause = Self::build_anchor_match_clause(predicate);

        let matches = self.execute_match(&clause, params)?;
        let mut ids = HashSet::with_capacity(matches.len());
        for m in matches {
            if let Some(id) = m.bindings.get(&anchor_alias) {
                ids.insert(*id);
            }
        }
        Ok(ids)
    }

    /// Extracts and validates the anchor alias from the first node in a MATCH predicate.
    fn resolve_anchor_alias(
        predicate: &crate::velesql::GraphMatchPredicate,
        from_aliases: &[String],
    ) -> Result<String> {
        let first_node = predicate.pattern.nodes.first().ok_or_else(|| {
            crate::error::Error::Config("MATCH predicate requires at least one node".to_string())
        })?;

        let anchor_alias = first_node.alias.clone().ok_or_else(|| {
            crate::error::Error::Config(
                "MATCH predicate in SELECT WHERE requires an alias on the first node, \
                 e.g. MATCH (d:Doc)-[:REL]->(x)"
                    .to_string(),
            )
        })?;

        // BUG-8: Check anchor alias against ALL aliases visible in scope.
        if !from_aliases.is_empty() && !from_aliases.iter().any(|a| a == &anchor_alias) {
            return Err(crate::error::Error::Config(format!(
                "MATCH predicate anchor alias '{}' must match one of the FROM/JOIN aliases: {:?}",
                anchor_alias, from_aliases
            )));
        }

        Ok(anchor_alias)
    }

    /// Builds a `MatchClause` that returns all bindings for anchor evaluation.
    fn build_anchor_match_clause(
        predicate: &crate::velesql::GraphMatchPredicate,
    ) -> crate::velesql::MatchClause {
        crate::velesql::MatchClause {
            patterns: vec![predicate.pattern.clone()],
            where_clause: None,
            return_clause: crate::velesql::ReturnClause {
                items: vec![crate::velesql::ReturnItem {
                    expression: "*".to_string(),
                    alias: None,
                }],
                order_by: None,
                // Internal anchor evaluation must not silently cap MATCH results.
                limit: Some(u64::MAX),
            },
        }
    }

    /// Dispatches the core vector / similarity / metadata query based on extracted components.
    ///
    /// Called from `execute_query_with_client` after query extraction and CBO planning.
    /// Handles all combinations of NEAR, similarity(), and metadata-only queries.
    /// Applies optional metadata post-filter to an already similarity-filtered result set.
    fn apply_optional_metadata_filter(
        filtered: Vec<SearchResult>,
        filter_cond: Option<&crate::velesql::Condition>,
        skip_metadata_prefilter_for_graph_or: bool,
        execution_limit: usize,
    ) -> Vec<SearchResult> {
        let Some(cond) = filter_cond else {
            return filtered;
        };
        if skip_metadata_prefilter_for_graph_or {
            return filtered;
        }
        let Some(metadata_cond) = Self::extract_metadata_filter(cond) else {
            return filtered;
        };
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond));
        filtered
            .into_iter()
            .filter(|r| match r.point.payload.as_ref() {
                Some(p) => filter.matches(p),
                None => filter.matches(&serde_json::Value::Null),
            })
            .take(execution_limit)
            .collect()
    }

    /// Applies all similarity cascade filters sequentially.
    fn apply_similarity_cascade(
        &self,
        candidates: Vec<SearchResult>,
        first_similarity: &(String, Vec<f32>, crate::velesql::CompareOp, f64),
        similarity_conditions: &[(String, Vec<f32>, crate::velesql::CompareOp, f64)],
        filter_k: usize,
    ) -> Vec<SearchResult> {
        let (field, vec, op, threshold) = first_similarity;
        let mut filtered =
            self.filter_by_similarity(candidates, field, vec, *op, *threshold, filter_k);
        for (sim_field, sim_vec, sim_op, sim_threshold) in similarity_conditions.iter().skip(1) {
            filtered = self.filter_by_similarity(
                filtered,
                sim_field,
                sim_vec,
                *sim_op,
                *sim_threshold,
                filter_k,
            );
        }
        filtered
    }

    /// Handles the `(NEAR vector, no similarity(), optional metadata filter)` path.
    #[allow(clippy::too_many_arguments)] // All arguments come from dispatch_vector_query.
    fn dispatch_near_with_filter(
        &self,
        vector: &[f32],
        cond: &crate::velesql::Condition,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
        search_opts: &QuerySearchOptions,
        cbo_strategy: crate::velesql::ExecutionStrategy,
        cbo_over_fetch: usize,
    ) -> Result<Vec<SearchResult>> {
        if let Some(text_query) = Self::extract_match_query(cond) {
            let fusion = search_opts.fusion_clause.as_ref();
            let vector_weight = fusion.and_then(|fc| fc.vector_weight).map(|w| {
                // SAFETY: f64 → f32 for API compat; weight is clamped 0.0–1.0.
                #[allow(clippy::cast_possible_truncation)]
                let w_f32 = w as f32;
                w_f32
            });
            let rrf_k = fusion.and_then(|fc| fc.k);
            // Bug #474: Extract co-occurring metadata filters (e.g. `category = 'tech'`)
            // before calling hybrid_search. Without this, metadata conditions alongside
            // MATCH are silently dropped.
            if let Some(metadata_cond) = Self::extract_metadata_filter(cond) {
                let filter =
                    crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond));
                return self.hybrid_search_with_filter(
                    vector,
                    &text_query,
                    execution_limit,
                    vector_weight,
                    &filter,
                    rrf_k,
                );
            }
            return self.hybrid_search(vector, &text_query, execution_limit, vector_weight, rrf_k);
        }
        let cbo_search_k = execution_limit
            .saturating_mul(cbo_over_fetch)
            .min(MAX_LIMIT);
        if skip_metadata_prefilter_for_graph_or {
            return self.search_with_opts(vector, execution_limit, search_opts);
        }
        if let Some(metadata_cond) = Self::extract_metadata_filter(cond) {
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond));
            return match cbo_strategy {
                crate::velesql::ExecutionStrategy::GraphFirst => {
                    Ok(self.scan_and_score_by_vector(&filter, vector, execution_limit))
                }
                _ => self.search_with_filter_and_opts(vector, cbo_search_k, &filter, search_opts),
            };
        }
        self.search_with_opts(vector, execution_limit, search_opts)
    }

    /// Handles the metadata-only (`(None, None, Some(cond))`) query path.
    fn dispatch_metadata_only(
        &self,
        cond: &crate::velesql::Condition,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
    ) -> Result<Vec<SearchResult>> {
        if let crate::velesql::Condition::Match(ref m) = cond {
            return self.text_search(&m.query, execution_limit);
        }
        let empty_filter =
            || crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] });
        if skip_metadata_prefilter_for_graph_or {
            return Ok(self.execute_scan_query(&empty_filter(), execution_limit));
        }
        if let Some(metadata_cond) = Self::extract_metadata_filter(cond) {
            // Fast path: use bitmap from secondary indexes (same mechanism as
            // search_with_filter). This handles AND conditions, Eq lookups, and
            // range queries via the bitmap infrastructure.
            let filter =
                crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond.clone()));
            if let Some(bitmap) = self.build_prefilter_bitmap(&filter) {
                if bitmap.is_empty() {
                    return Ok(Vec::new());
                }
                // Convert bitmap to ID list and scan with filter
                let candidate_ids: Vec<u64> = bitmap.iter().map(u64::from).collect();
                if candidate_ids.len() <= execution_limit.saturating_mul(50).max(1000) {
                    return Ok(self.scan_candidate_ids_metadata(
                        &candidate_ids,
                        &filter,
                        execution_limit,
                    ));
                }
                // Too many bitmap hits — fall through to scan with early exit
            }

            tracing::debug!("dispatch_metadata_only: trying indexed path");
            if let Some(indexed) =
                self.execute_indexed_metadata_query(&metadata_cond, execution_limit)
            {
                tracing::debug!("dispatch_metadata_only: indexed path succeeded");
                return Ok(indexed);
            }
            tracing::debug!("dispatch_metadata_only: indexed path returned None, trying BM25");

            // Try BM25 text search for LIKE conditions before falling back to full scan.
            // When a LIKE pattern contains a word-like substring (e.g. `%google%`),
            // BM25 can narrow candidates significantly faster than a sequential scan.
            if let Some(like_results) = self.try_like_via_text_index(cond, execution_limit) {
                return Ok(like_results);
            }

            let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond));
            return Ok(self.execute_scan_query(&filter, execution_limit));
        }
        Ok(self.execute_scan_query(&empty_filter(), execution_limit))
    }

    /// Attempts to accelerate a LIKE condition using the BM25 text index.
    ///
    /// Extracts the word-like core from a `%word%` pattern and queries BM25
    /// for candidate document IDs. The full condition is then post-filtered
    /// over those candidates instead of scanning the entire collection.
    ///
    /// Returns `Some(results)` only when BM25 found enough candidates to
    /// fill the limit. When BM25 returns fewer matches than requested, the
    /// result set may be incomplete (BM25 tokenization differs from LIKE
    /// substring matching), so we return `None` to let the caller fall
    /// through to a full sequential scan.
    ///
    /// Returns `None` when:
    /// - No LIKE condition is found in the condition tree
    /// - The extracted word is too short (< 3 chars) for meaningful BM25 lookup
    /// - BM25 returns no candidates (fall through to sequential scan)
    /// - BM25 candidates yield fewer than `limit` matches (incomplete set)
    fn try_like_via_text_index(
        &self,
        cond: &crate::velesql::Condition,
        limit: usize,
    ) -> Option<Vec<SearchResult>> {
        let pattern = Self::extract_like_pattern(cond)?;

        // Extract the word-like core from the pattern (strip leading/trailing %).
        let word = pattern.trim_matches('%');
        if word.is_empty() || word.len() < 3 {
            return None;
        }

        // Use BM25 text index to find candidates (over-fetch 10× for post-filter headroom).
        let text_results = self.text_index.search(word, limit.saturating_mul(10));
        if text_results.is_empty() {
            return None;
        }

        let candidate_ids: Vec<u64> = text_results.iter().map(|(id, _)| *id).collect();
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));

        let mut results = Vec::new();
        for point in self.get(&candidate_ids).into_iter().flatten() {
            let payload = point.payload.clone().unwrap_or(serde_json::Value::Null);
            if filter.matches(&payload) {
                results.push(SearchResult::new(point, 1.0));
                if results.len() >= limit {
                    break;
                }
            }
        }

        // Only return BM25 results when we filled the limit — otherwise the
        // result set may be incomplete because BM25 tokenization differs from
        // LIKE substring matching (e.g., "analytics.google.com" won't match
        // BM25 for "google" but should match LIKE '%google%').
        if results.len() >= limit {
            Some(results)
        } else {
            None // Fall through to full sequential scan
        }
    }

    /// Recursively extracts the first LIKE pattern from a condition tree.
    fn extract_like_pattern(cond: &crate::velesql::Condition) -> Option<String> {
        match cond {
            crate::velesql::Condition::Like(like) => Some(like.pattern.clone()),
            crate::velesql::Condition::And(left, right) => {
                Self::extract_like_pattern(left).or_else(|| Self::extract_like_pattern(right))
            }
            crate::velesql::Condition::Group(inner) => Self::extract_like_pattern(inner),
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)] // All arguments come from query extraction in the caller.
    pub(super) fn dispatch_vector_query(
        &self,
        vector_search: Option<&Vec<f32>>,
        first_similarity: Option<&(String, Vec<f32>, crate::velesql::CompareOp, f64)>,
        similarity_conditions: &[(String, Vec<f32>, crate::velesql::CompareOp, f64)],
        filter_condition: Option<&crate::velesql::Condition>,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
        search_opts: &QuerySearchOptions,
        cbo_strategy: crate::velesql::ExecutionStrategy,
        cbo_over_fetch: usize,
    ) -> Result<Vec<SearchResult>> {
        match (vector_search, first_similarity, filter_condition) {
            // similarity() with optional NEAR vector and optional metadata filter
            (search_vec, Some(sim), filter_cond) => self.dispatch_similarity_query(
                search_vec.map(Vec::as_slice),
                sim,
                similarity_conditions,
                filter_cond,
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
                search_opts,
            ),
            // NEAR + metadata filter (no similarity threshold)
            (Some(vector), None, Some(cond)) => self.dispatch_near_with_filter(
                vector,
                cond,
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
                search_opts,
                cbo_strategy,
                cbo_over_fetch,
            ),
            // Pure NEAR (no filter, no similarity threshold)
            (Some(vector), None, None) => {
                self.dispatch_pure_near(vector, execution_limit, search_opts)
            }
            // Metadata-only
            (None, None, Some(cond)) => self.dispatch_metadata_only(
                cond,
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
            ),
            // SELECT * (no WHERE)
            (None, None, None) => Ok(self.execute_scan_query(
                &crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] }),
                execution_limit,
            )),
        }
    }

    /// Handles the similarity() path with optional NEAR vector and optional metadata filter.
    #[allow(clippy::too_many_arguments)] // All arguments come from dispatch_vector_query.
    fn dispatch_similarity_query(
        &self,
        search_vector: Option<&[f32]>,
        sim: &(String, Vec<f32>, crate::velesql::CompareOp, f64),
        similarity_conditions: &[(String, Vec<f32>, crate::velesql::CompareOp, f64)],
        filter_cond: Option<&crate::velesql::Condition>,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
        search_opts: &QuerySearchOptions,
    ) -> Result<Vec<SearchResult>> {
        let k = execution_limit
            .saturating_mul(10 * similarity_conditions.len().max(1))
            .min(MAX_LIMIT);
        let search_vec = search_vector.unwrap_or(&sim.1);
        let candidates = self.search_with_opts(search_vec, k, search_opts)?;
        let filtered = self.apply_similarity_cascade(
            candidates,
            sim,
            similarity_conditions,
            execution_limit.saturating_mul(2),
        );
        Ok(Self::apply_optional_metadata_filter(
            filtered,
            filter_cond,
            skip_metadata_prefilter_for_graph_or,
            execution_limit,
        ))
    }

    /// Handles the pure NEAR path (no similarity threshold, no metadata filter).
    fn dispatch_pure_near(
        &self,
        vector: &[f32],
        execution_limit: usize,
        search_opts: &QuerySearchOptions,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_opts(vector, execution_limit, search_opts)
    }
}
