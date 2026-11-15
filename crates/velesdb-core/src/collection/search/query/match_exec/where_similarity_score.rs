//! Row scores for a GraphFirst MATCH whose WHERE holds a similarity leaf.
//!
//! GraphFirst checks `similarity(alias.field, $v) <op> t` as one more WHERE
//! leaf: a boolean that keeps or drops the row. The score itself has to be
//! carried on the row, or `ORDER BY similarity()` compares nothing,
//! `RETURN similarity()` projects nothing, and the row falls back to the
//! depth score — while VectorFirst, chosen when the leaf targets the start
//! node, reports the real similarity. This pass gives GraphFirst rows the
//! same score VectorFirst rows get: the similarity of the node the first
//! similarity leaf targets.

use super::where_eval::{resolve_query_vector, resolve_target_id};
use super::{MatchResult, MatchStorageGuards};
use crate::collection::types::Collection;
use crate::error::Result;
use crate::storage::VectorStorage;
use crate::velesql::{Condition, MatchClause, SimilarityCondition};
use std::collections::HashMap;

/// The first `similarity()` leaf of a WHERE tree, left to right — the leaf
/// both strategies score rows by.
pub(super) fn first_similarity_condition(condition: &Condition) -> Option<&SimilarityCondition> {
    match condition {
        Condition::Similarity(sim) => Some(sim),
        Condition::And(left, right) | Condition::Or(left, right) => {
            first_similarity_condition(left).or_else(|| first_similarity_condition(right))
        }
        Condition::Not(inner) | Condition::Group(inner) => first_similarity_condition(inner),
        _ => None,
    }
}

impl Collection {
    /// Sets each row's score to the similarity of the node targeted by the
    /// WHERE clause's first similarity leaf, and re-projects RETURN so
    /// `similarity()` carries it. A row whose target has no vector, or one of
    /// another dimension, keeps its score: the leaf could not have scored it.
    ///
    /// # Errors
    ///
    /// Returns an error when the leaf's vector parameter is missing or not an
    /// array, or when vector storage cannot be read.
    pub(super) fn score_rows_by_where_similarity(
        &self,
        results: &mut [MatchResult],
        match_clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        guards: &MatchStorageGuards<'_>,
    ) -> Result<()> {
        let Some(sim) = match_clause
            .where_clause
            .as_ref()
            .and_then(first_similarity_condition)
        else {
            return Ok(());
        };
        let query_vector = resolve_query_vector(&sim.vector, params)?;
        if query_vector.is_empty() {
            return Ok(());
        }
        for row in results {
            let target = resolve_target_id(&sim.field, Some(&row.bindings), row.node_id);
            let Some(node_vector) = guards.vector_guard.retrieve(target)? else {
                continue;
            };
            if node_vector.len() != query_vector.len() {
                continue;
            }
            let score = guards.metric.calculate(&node_vector, &query_vector);
            row.score = Some(score);
            row.projected = self.project_properties_with_score(
                &row.bindings,
                &row.edge_bindings,
                &row.edge_paths,
                &match_clause.return_clause,
                Some(score),
                &guards.payload_memo,
            );
        }
        Ok(())
    }
}
