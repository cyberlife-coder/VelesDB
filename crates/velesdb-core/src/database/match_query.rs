//! A bare MATCH clause run against a named collection: what REST `/match`
//! and the database-level SDK calls run.
//!
//! A `MATCH` statement sent to [`Database::execute_query`] is gated, routed
//! and given its `@collection` sources by the statement path. A surface that
//! holds a parsed [`MatchClause`] and a collection name instead — REST
//! `/match` — goes through [`Database::match_query`], which applies the same
//! three steps: the read gate on the target and on every annotated
//! collection, the annotations resolved before any lock, and the ordered
//! planner every MATCH surface shares.

use std::collections::HashMap;

use super::read_authorization::GateDecisions;
use super::Database;
use crate::collection::search::query::match_exec::MatchResult;
use crate::error::Result;
use crate::velesql::MatchClause;

/// A query vector the rows of a MATCH are scored against (REST `/match`'s
/// `vector` and `threshold`): each row's last node is scored, rows past the
/// threshold are dropped and the rest ordered by score before any
/// `ORDER BY`.
#[derive(Debug, Clone, Copy)]
pub struct MatchScoring<'v> {
    /// The query vector, of the collection's dimension.
    pub vector: &'v [f32],
    /// Kept rows score at least this (at most, under a distance metric).
    pub threshold: f32,
}

impl Database {
    /// Runs `clause` on the collection `collection`, as a `MATCH` statement
    /// with that target would run: the read gate admits the target and every
    /// `@collection` the clause names (each unscoped), the annotated nodes
    /// read their collection's properties during the walk, and the rows are
    /// ordered and limited as on `/query`, `search.max_results` included.
    ///
    /// # Errors
    ///
    /// The read gate's refusal, [`crate::error::Error::CollectionNotFound`]
    /// for the target or an annotation naming no collection, or any error of
    /// the MATCH itself (validation, guard rails, a dimension mismatch).
    pub fn match_query(
        &self,
        collection: &str,
        clause: &MatchClause,
        params: &HashMap<String, serde_json::Value>,
        scoring: Option<MatchScoring<'_>>,
    ) -> Result<Vec<MatchResult>> {
        self.authorize_match_clause(Some(collection), Some(clause), &GateDecisions::default())?;
        let target = self.resolve_collection(collection)?;
        let sources = self.match_sources(Some(clause))?;
        let mut rows = match scoring {
            Some(scoring) => target.execute_match_with_similarity_reading(
                clause,
                scoring.vector,
                scoring.threshold,
                params,
                &sources,
            ),
            None => target.match_query_ordered_reading(clause, params, &sources),
        }?;
        // `search.max_results` bounds what a caller receives (#2591).
        rows.truncate(self.config.search.max_results);
        Ok(rows)
    }
}
