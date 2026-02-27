use super::*;

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
        from_alias: Option<&str>,
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

        if let Some(from_alias) = from_alias {
            if from_alias != anchor_alias {
                return Err(crate::error::Error::Config(format!(
                    "MATCH predicate anchor alias '{}' must match FROM alias '{}'",
                    anchor_alias, from_alias
                )));
            }
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
}
