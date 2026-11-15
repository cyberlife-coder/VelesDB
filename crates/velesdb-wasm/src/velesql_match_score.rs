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
//!   row's last node is scored, rows past the threshold are dropped
//!   (`>=` it, or `<=` under a distance metric), and rows are ordered by
//!   score before any `ORDER BY`.

use velesdb_core::velesql::{CompareOp, Condition, SimilarityCondition};

use crate::velesql_match_nodes::MatchNodes;
use crate::velesql_value::{resolve_vector, Params};

/// REST `/match`'s query vector and threshold.
#[derive(Debug, Clone)]
pub(crate) struct MatchScoring {
    pub vector: Vec<f32>,
    /// Kept rows score at least this (at most, under a distance metric).
    /// REST's default is `0.0`.
    pub threshold: Option<f32>,
}

impl MatchScoring {
    /// Refuses what REST `/match` refuses: a threshold outside `[0, 1]`, a
    /// vector whose dimension is not the collection's.
    pub(crate) fn validate(&self, nodes: &MatchNodes<'_>) -> Result<(), String> {
        if let Some(t) = self.threshold {
            if !(0.0..=1.0).contains(&t) {
                return Err(format!("MATCH threshold must be in [0.0, 1.0], got {t}"));
            }
        }
        let Some((dimension, _)) = nodes.vector_space() else {
            return Err("MATCH with a query vector needs a collection with vectors".to_string());
        };
        if self.vector.len() != dimension {
            return Err(format!(
                "MATCH query vector dimension mismatch: expected {dimension}, got {}",
                self.vector.len()
            ));
        }
        Ok(())
    }

    /// The score of `node` and whether it passes the threshold.
    pub(crate) fn score(&self, nodes: &MatchNodes<'_>, node: u64) -> Option<f32> {
        let (_, metric) = nodes.vector_space()?;
        let score = metric.calculate(nodes.vector(node)?, &self.vector);
        let threshold = self.threshold.unwrap_or(0.0);
        let passes = if metric.higher_is_better() {
            score >= threshold
        } else {
            score <= threshold
        };
        passes.then_some(score)
    }
}

/// One bound node, as WHERE sees it.
pub(crate) struct BoundNode<'b> {
    pub alias: &'b str,
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
    other_leaf: &dyn Fn(&Condition) -> Result<Option<bool>, String>,
) -> Result<bool, String> {
    Ok(where_truth(cond, bound, nodes, params, other_leaf)? == Some(true))
}

/// The three-valued truth behind [`where_holds`]. A `similarity()` leaf
/// whose node has no vector of the query's dimension is UNKNOWN, as in core.
fn where_truth(
    cond: &Condition,
    bound: &[BoundNode<'_>],
    nodes: &MatchNodes<'_>,
    params: &Params,
    other_leaf: &dyn Fn(&Condition) -> Result<Option<bool>, String>,
) -> Result<Option<bool>, String> {
    use crate::velesql_where::{and, or};
    match cond {
        Condition::And(l, r) => Ok(and(
            where_truth(l, bound, nodes, params, other_leaf)?,
            where_truth(r, bound, nodes, params, other_leaf)?,
        )),
        Condition::Or(l, r) => Ok(or(
            where_truth(l, bound, nodes, params, other_leaf)?,
            where_truth(r, bound, nodes, params, other_leaf)?,
        )),
        Condition::Not(inner) => {
            Ok(where_truth(inner, bound, nodes, params, other_leaf)?.map(|known| !known))
        }
        Condition::Group(inner) => where_truth(inner, bound, nodes, params, other_leaf),
        Condition::Similarity(sim) => Ok(leaf_score(sim, bound, nodes, params)?
            .map(|(score, higher_is_better)| compare(sim, score, higher_is_better))),
        leaf => other_leaf(leaf),
    }
}

/// The row's score from its first `similarity()` leaf, if WHERE has one and
/// its node has a vector.
pub(crate) fn where_score(
    cond: Option<&Condition>,
    bound: &[BoundNode<'_>],
    nodes: &MatchNodes<'_>,
    params: &Params,
) -> Result<Option<f32>, String> {
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
) -> Result<Option<(f32, bool)>, String> {
    let query = resolve_vector(&sim.vector, params)?;
    let target = sim
        .field
        .split_once('.')
        .and_then(|(alias, _)| bound.iter().find(|b| b.alias == alias))
        .or_else(|| bound.last());
    let Some(target) = target else {
        return Ok(None);
    };
    let (Some(vector), Some((_, metric))) = (nodes.vector(target.id), nodes.vector_space()) else {
        return Ok(None);
    };
    if vector.len() != query.len() {
        return Ok(None);
    }
    Ok(Some((
        metric.calculate(vector, &query),
        metric.higher_is_better(),
    )))
}

/// Core's `compare_score`.
fn compare(sim: &SimilarityCondition, score: f32, higher_is_better: bool) -> bool {
    #[allow(clippy::cast_possible_truncation)]
    let threshold = sim.threshold as f32;
    let op = if higher_is_better {
        sim.operator
    } else {
        match sim.operator {
            CompareOp::Gt => CompareOp::Lt,
            CompareOp::Lt => CompareOp::Gt,
            CompareOp::Gte => CompareOp::Lte,
            CompareOp::Lte => CompareOp::Gte,
            other => other,
        }
    };
    match op {
        CompareOp::Gt => score > threshold,
        CompareOp::Gte => score >= threshold,
        CompareOp::Lt => score < threshold,
        CompareOp::Lte => score <= threshold,
        CompareOp::Eq => (score - threshold).abs() < f32::EPSILON,
        CompareOp::NotEq => (score - threshold).abs() >= f32::EPSILON,
        _ => false,
    }
}

#[cfg(test)]
#[path = "velesql_match_score_tests.rs"]
mod tests;
