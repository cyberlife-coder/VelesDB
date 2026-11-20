//! A statement's query vectors, checked against this collection once, before
//! any plan is chosen or any row is read ([`crate::velesql::QueryVectorTarget`]).
//!
//! Every SELECT, aggregate and MATCH entry point calls one of these before
//! it dispatches: [`Collection::execute_query_traced`] (SELECT, each operand
//! of a compound), [`Collection::execute_aggregate`], and the three MATCH
//! funnels — the planner's `dispatch_match_strategy` (SQL `/query` and
//! `match_query_ordered`), `execute_match_with_context` (`execute_match`) and
//! `execute_match_with_similarity_reading` (REST `/match` with a vector).

use std::collections::HashMap;

use crate::collection::types::Collection;
use crate::error::Result;
use crate::velesql::{MatchClause, SelectStatement};

impl Collection {
    /// Refuses a SELECT whose query vectors this collection cannot score
    /// ([`crate::velesql::QueryVectorTarget::check_select`]).
    ///
    /// # Errors
    ///
    /// [`crate::velesql::QueryVectorTarget::check`]'s refusal.
    pub(crate) fn check_select_query_vectors(
        &self,
        stmt: &SelectStatement,
        params: &HashMap<String, serde_json::Value>,
    ) -> Result<()> {
        self.storage
            .config
            .read()
            .query_vector_target()
            .check_select(stmt, params)
    }

    /// Refuses a MATCH whose query vectors this collection cannot score
    /// ([`crate::velesql::QueryVectorTarget::check_match`]), the scoring
    /// vector of REST `/match` (`scoring`) included.
    ///
    /// # Errors
    ///
    /// [`crate::velesql::QueryVectorTarget::check_match`]'s refusal.
    pub(crate) fn check_match_query_vectors(
        &self,
        clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        scoring: Option<&[f32]>,
    ) -> Result<()> {
        self.storage
            .config
            .read()
            .query_vector_target()
            .check_match(clause, params, scoring.map(<[f32]>::len))
    }
}
