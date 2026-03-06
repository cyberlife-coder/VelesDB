use super::{Collection, HashSet, Result, SearchResult, MAX_LIMIT};

impl Collection {
    pub(super) fn execute_indexed_metadata_query(
        &self,
        cond: &crate::velesql::Condition,
        execution_limit: usize,
    ) -> Option<Vec<SearchResult>> {
        let (field_name, key) = Self::extract_index_lookup_condition(cond)?;
        let ids = self.secondary_index_lookup(&field_name, &key)?;
        let filter = crate::filter::Filter::new(crate::filter::Condition::from(cond.clone()));
        let mut results = Vec::new();
        for point in self.get(&ids).into_iter().flatten() {
            let payload = point.payload.clone().unwrap_or(serde_json::Value::Null);
            if filter.matches(&payload) {
                results.push(SearchResult::new(point, 0.0));
                if results.len() >= execution_limit {
                    break;
                }
            }
        }
        Some(results)
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

    pub(crate) fn evaluate_graph_match_anchor_ids(
        &self,
        predicate: &crate::velesql::GraphMatchPredicate,
        params: &std::collections::HashMap<String, serde_json::Value>,
        from_aliases: &[String],
    ) -> Result<HashSet<u64>> {
        let pattern = &predicate.pattern;
        let first_node = pattern.nodes.first().ok_or_else(|| {
            crate::error::Error::Config("MATCH predicate requires at least one node".to_string())
        })?;

        let anchor_alias = first_node.alias.clone().ok_or_else(|| {
            crate::error::Error::Config(
                "MATCH predicate in SELECT WHERE requires an alias on the first node, e.g. MATCH (d:Doc)-[:REL]->(x)"
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

        let clause = crate::velesql::MatchClause {
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
        };

        let matches = self.execute_match(&clause, params)?;
        let mut ids = HashSet::with_capacity(matches.len());
        for m in matches {
            if let Some(id) = m.bindings.get(&anchor_alias) {
                ids.insert(*id);
            }
        }
        Ok(ids)
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
    fn dispatch_near_with_filter(
        &self,
        vector: &[f32],
        cond: &crate::velesql::Condition,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
        cbo_strategy: crate::velesql::ExecutionStrategy,
        cbo_over_fetch: usize,
    ) -> Result<Vec<SearchResult>> {
        if let Some(text_query) = Self::extract_match_query(cond) {
            return self.hybrid_search(vector, &text_query, execution_limit, None);
        }
        let cbo_search_k = execution_limit
            .saturating_mul(cbo_over_fetch)
            .min(MAX_LIMIT);
        if skip_metadata_prefilter_for_graph_or {
            return self.search(vector, execution_limit);
        }
        if let Some(metadata_cond) = Self::extract_metadata_filter(cond) {
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond));
            return match cbo_strategy {
                crate::velesql::ExecutionStrategy::GraphFirst => {
                    Ok(self.scan_and_score_by_vector(&filter, vector, execution_limit))
                }
                _ => self.search_with_filter(vector, cbo_search_k, &filter),
            };
        }
        self.search(vector, execution_limit)
    }

    /// Handles the metadata-only (`(None, None, Some(cond))`) query path.
    fn dispatch_metadata_only(
        &self,
        cond: &crate::velesql::Condition,
        execution_limit: usize,
        skip_metadata_prefilter_for_graph_or: bool,
    ) -> Vec<SearchResult> {
        if let crate::velesql::Condition::Match(ref m) = cond {
            return self.text_search(&m.query, execution_limit);
        }
        let empty_filter =
            || crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] });
        if skip_metadata_prefilter_for_graph_or {
            return self.execute_scan_query(&empty_filter(), execution_limit);
        }
        if let Some(metadata_cond) = Self::extract_metadata_filter(cond) {
            if let Some(indexed) =
                self.execute_indexed_metadata_query(&metadata_cond, execution_limit)
            {
                return indexed;
            }
            let filter = crate::filter::Filter::new(crate::filter::Condition::from(metadata_cond));
            return self.execute_scan_query(&filter, execution_limit);
        }
        self.execute_scan_query(&empty_filter(), execution_limit)
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
        ef_search: Option<usize>,
        cbo_strategy: crate::velesql::ExecutionStrategy,
        cbo_over_fetch: usize,
    ) -> Result<Vec<SearchResult>> {
        let results = match (vector_search, first_similarity, filter_condition) {
            // similarity() — search by first vector, cascade-filter, optional metadata
            (None, Some(sim), filter_cond) => {
                let k = execution_limit
                    .saturating_mul(10 * similarity_conditions.len().max(1))
                    .min(MAX_LIMIT);
                let candidates = self.search(&sim.1, k)?;
                let filtered = self.apply_similarity_cascade(
                    candidates,
                    sim,
                    similarity_conditions,
                    execution_limit.saturating_mul(2),
                );
                Self::apply_optional_metadata_filter(
                    filtered,
                    filter_cond,
                    skip_metadata_prefilter_for_graph_or,
                    execution_limit,
                )
            }
            // NEAR + similarity() + optional metadata
            (Some(vector), Some(sim), filter_cond) => {
                let k = execution_limit
                    .saturating_mul(10 * similarity_conditions.len().max(1))
                    .min(MAX_LIMIT);
                let candidates = self.search(vector, k)?;
                let filtered = self.apply_similarity_cascade(
                    candidates,
                    sim,
                    similarity_conditions,
                    execution_limit.saturating_mul(2),
                );
                Self::apply_optional_metadata_filter(
                    filtered,
                    filter_cond,
                    skip_metadata_prefilter_for_graph_or,
                    execution_limit,
                )
            }
            // NEAR + metadata filter (no similarity threshold)
            (Some(vector), None, Some(cond)) => self.dispatch_near_with_filter(
                vector,
                cond,
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
                cbo_strategy,
                cbo_over_fetch,
            )?,
            // Pure NEAR (no filter, no similarity threshold)
            (Some(vector), _, None) => {
                if let Some(ef) = ef_search {
                    self.search_with_ef(vector, execution_limit, ef)?
                } else {
                    self.search(vector, execution_limit)?
                }
            }
            // Metadata-only
            (None, None, Some(cond)) => self.dispatch_metadata_only(
                cond,
                execution_limit,
                skip_metadata_prefilter_for_graph_or,
            ),
            // SELECT * (no WHERE)
            (None, None, None) => self.execute_scan_query(
                &crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] }),
                execution_limit,
            ),
        };
        Ok(results)
    }
}
