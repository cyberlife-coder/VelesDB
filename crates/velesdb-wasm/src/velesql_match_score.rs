//! `similarity()` in a browser MATCH, with core's MATCH semantics.
//!
//! A collection's points are its MATCH nodes ([`MatchNodes`]), so their
//! vectors score `similarity()` here as core scores them:
//!
//! - **WHERE.** `similarity(alias.field, $v) <op> t` scores the node `alias`
//!   binds; a bare field scores the row's last node (core's traversal
//!   target). Comparison follows core's `compare_score`: under a distance
//!   metric `similarity > t` means `distance < t`. A node without a vector,
//!   or one of another dimension, makes the leaf UNKNOWN: neither it nor its
//!   negation admits the row. The row's score
//!   is its first similarity leaf's (core #2787).
//! - **Query vector** (`executeMatch`'s `vector`, REST `/match`'s): each
//!   row's last node is scored, rows past the threshold, when one is given,
//!   are dropped by core's rule
//!   ([`velesdb_core::velesql::match_threshold_keeps`]), and rows are
//!   ordered by score before any `ORDER BY`.
//!
//! Every query vector — this one and each `similarity()`'s — is checked
//! against the collection before the walk, by core's
//! [`velesdb_core::velesql::QueryVectorTarget`] (`velesql_match`).

use velesdb_core::velesql::{Condition, MatchClause, SimilarityCondition};

use crate::exec_error::ExecError;
use crate::velesql_match_nodes::MatchNodes;
use crate::velesql_value::Params;

/// REST `/match`'s query vector and threshold, built only through
/// [`MatchScoring::from_request`], which applies core's threshold rule once.
#[derive(Debug, Clone)]
pub(crate) struct MatchScoring {
    vector: Vec<f32>,
    /// Kept rows score at least this (at most, under a distance metric).
    /// `None` keeps every scored row, as on REST.
    threshold: Option<f32>,
}

impl MatchScoring {
    /// The scoring of an `executeMatch` call from its optional query
    /// `vector` and `threshold`, under core's rule
    /// ([`velesdb_core::velesql::validate_match_threshold`]), as REST
    /// `/match` builds its own: `None` without a vector; a threshold must be
    /// finite and come with a vector.
    ///
    /// # Errors
    ///
    /// Core's `Error::Query` (`VELES-010`) for a threshold it cannot apply.
    pub(crate) fn from_request(
        vector: Option<Vec<f32>>,
        threshold: Option<f32>,
    ) -> Result<Option<Self>, ExecError> {
        velesdb_core::velesql::validate_match_threshold(vector.as_deref(), threshold)?;
        Ok(vector.map(|vector| Self { vector, threshold }))
    }

    /// The query vector, which the walk checks against the collection
    /// before scoring any row (`velesql_match`).
    pub(crate) fn vector(&self) -> &[f32] {
        &self.vector
    }

    /// The score of `node`, when it passes the threshold
    /// ([`velesdb_core::velesql::match_threshold_keeps`]).
    pub(crate) fn score(&self, nodes: &MatchNodes<'_>, node: u64) -> Option<f32> {
        let (_, metric) = nodes.vector_space()?;
        let score = metric.checked_calculate(nodes.vector(node)?, &self.vector)?;
        velesdb_core::velesql::match_threshold_keeps(
            self.threshold,
            score,
            metric.higher_is_better(),
        )
        .then_some(score)
    }
}

/// Refuses, before the walk, a query vector the collection cannot score —
/// REST `/match`'s and every `similarity()`'s — by core's rule
/// ([`velesdb_core::velesql::QueryVectorTarget`]): a collection without
/// vectors (`SearchNotSupported`, VELES-015), then another dimension
/// (`DimensionMismatch`, VELES-004). A walk that finds no row must not turn
/// the refusal into an empty result.
pub(crate) fn check_query_vectors(
    collection: &str,
    nodes: &MatchNodes<'_>,
    clause: &MatchClause,
    params: &Params,
    scoring: Option<&MatchScoring>,
) -> Result<(), ExecError> {
    crate::store_search::query_vector_target(collection, nodes.store_dimension()).check_match(
        clause,
        params,
        scoring.map(|scoring| scoring.vector().len()),
    )?;
    Ok(())
}

/// One bound node, as WHERE sees it, in pattern order: its alias (`None`
/// for an unnamed node, which no column can address) and its id. The last
/// is the walk's target, which a bare column scores.
pub(crate) struct BoundNode<'b> {
    pub alias: Option<&'b str>,
    pub id: u64,
}

/// Evaluates a MATCH WHERE whose leaves may include `similarity()`.
///
/// `similarity()` leaves are scored here; every other leaf goes to
/// `other_leaf` (the payload evaluator's three-valued truth), so the two
/// never disagree on the rest of the grammar. The tree folds in SQL's
/// three-valued logic, as core's MATCH WHERE does, and the row is admitted
/// only when it is known TRUE.
pub(crate) fn where_holds(
    cond: &Condition,
    bound: &[BoundNode<'_>],
    nodes: &MatchNodes<'_>,
    params: &Params,
    other_leaf: &dyn Fn(&Condition) -> Result<Option<bool>, ExecError>,
) -> Result<bool, ExecError> {
    Ok(where_truth(cond, bound, nodes, params, other_leaf)? == Some(true))
}

/// The three-valued truth behind [`where_holds`]. A `similarity()` leaf
/// whose node has no vector of the query's dimension is UNKNOWN, as in core.
fn where_truth(
    cond: &Condition,
    bound: &[BoundNode<'_>],
    nodes: &MatchNodes<'_>,
    params: &Params,
    other_leaf: &dyn Fn(&Condition) -> Result<Option<bool>, ExecError>,
) -> Result<Option<bool>, ExecError> {
    crate::velesql_where::fold_truth(cond, &|leaf| match leaf {
        Condition::Similarity(sim) => Ok(leaf_score(sim, bound, nodes, params)?.map(
            |(score, higher_is_better)| {
                velesdb_core::velesql::similarity_holds(
                    sim.operator,
                    score,
                    sim.threshold,
                    higher_is_better,
                )
            },
        )),
        other => other_leaf(other),
    })
}

/// The row's score from its first `similarity()` leaf, if WHERE has one and
/// its node has a vector.
pub(crate) fn where_score(
    cond: Option<&Condition>,
    bound: &[BoundNode<'_>],
    nodes: &MatchNodes<'_>,
    params: &Params,
) -> Result<Option<f32>, ExecError> {
    let Some(sim) = cond.and_then(first_similarity) else {
        return Ok(None);
    };
    Ok(leaf_score(sim, bound, nodes, params)?.map(|(score, _)| score))
}

fn first_similarity(cond: &Condition) -> Option<&SimilarityCondition> {
    match cond {
        Condition::Similarity(sim) => Some(sim),
        Condition::And(l, r) | Condition::Or(l, r) => {
            first_similarity(l).or_else(|| first_similarity(r))
        }
        Condition::Not(inner) | Condition::Group(inner) => first_similarity(inner),
        _ => None,
    }
}

/// The leaf's raw score and the metric's direction; `None` when the target
/// node has no vector of the query's dimension.
fn leaf_score(
    sim: &SimilarityCondition,
    bound: &[BoundNode<'_>],
    nodes: &MatchNodes<'_>,
    params: &Params,
) -> Result<Option<(f32, bool)>, ExecError> {
    let query = velesdb_core::velesql::bind_vector(&sim.vector, params)?;
    let target = sim
        .field
        .split_once('.')
        .and_then(|(alias, _)| bound.iter().find(|b| b.alias == Some(alias)))
        .or_else(|| bound.last());
    let Some(target) = target else {
        return Ok(None);
    };
    let (Some(vector), Some((_, metric))) = (nodes.vector(target.id), nodes.vector_space()) else {
        return Ok(None);
    };
    Ok(metric
        .checked_calculate(vector, &query)
        .map(|score| (score, metric.higher_is_better())))
}

#[cfg(test)]
#[path = "velesql_match_score_tests.rs"]
mod tests;
