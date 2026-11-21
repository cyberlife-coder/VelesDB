//! The vector predicates of a WHERE clause, as every engine reads them —
//! core's SELECT pipeline and the WASM executor alike.
//!
//! - [`validate_vector_predicates`] refuses the shapes no executor can run as
//!   written, so both engines refuse the same queries with the same words.
//! - [`payload_vector`] reads a *named* vector — `similarity(embedding, $v)`
//!   scores the numeric array the point's payload holds under `embedding`;
//!   `similarity(vector, $v)` scores the point's own vector.

use crate::error::{Error, Result};

use super::condition_tree::{any_subtree, count_matching_leaves, is_vector_leaf};
use super::text_match::{and_required_text_matches, ranking_near};
use super::Condition;

/// The column naming a point's own vector: `similarity(vector, $v)`.
pub const PRIMARY_VECTOR_FIELD: &str = "vector";

/// The score of a row no vector predicate scored: a metadata match.
pub const METADATA_MATCH_SCORE: f32 = 1.0;

/// Refuses a WHERE clause whose vector predicates no executor can run as
/// written:
///
/// - several `similarity()` under one `OR` (it would take a union of vector
///   searches); several under `AND` filter in cascade and are accepted;
/// - more than one `SPARSE_NEAR` (a query runs one sparse branch), or one
///   under `NOT` (it would rank the rows it negates);
/// - a `NEAR_FUSED` that is not the only vector predicate, or that sits
///   under `OR` / `NOT` (the fusion executor would not reach it);
/// - a text `MATCH` that would rank beside a ranking no executor fuses it
///   with — a `SPARSE_NEAR`, a `NEAR_FUSED`, a `NEAR` beside a
///   `similarity()` threshold — or a second one that would rank.
///
/// # Errors
///
/// [`Error::Query`] naming the refused shape.
pub fn validate_vector_predicates(condition: &Condition) -> Result<()> {
    if count_matching_leaves(condition, is_vector_leaf) > 1
        && has_multiple_similarity_in_or(condition)
    {
        return Err(Error::Query(
            "Multiple similarity() conditions in OR are not supported. \
            Use AND to apply filters sequentially, or split into separate queries."
                .to_string(),
        ));
    }
    reject_unexecutable_sparse_search(condition)?;
    reject_unexecutable_fused_search(condition)?;
    reject_unexecutable_text_match(condition)
}

/// The vector the payload holds under `field`, for a named-vector
/// `similarity(field, $v)`: `None` for an absent or null field, including
/// the nulls a JOIN supplies on its unmatched side.
///
/// # Errors
///
/// [`Error::Query`] when a non-null field holds something other than an
/// array of numbers.
pub fn payload_vector(payload: &serde_json::Value, field: &str) -> Result<Option<Vec<f32>>> {
    let Some(value) = super::row_field(payload, field).filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let Some(array) = value.as_array() else {
        return Err(Error::Query(format!(
            "similarity() field '{field}' is not a numeric array in payload."
        )));
    };
    #[allow(clippy::cast_possible_truncation)]
    // Reason: JSON vector components are f64 embeddings; the vector store
    // operates on f32 precision.
    let vector: Option<Vec<f32>> = array.iter().map(|v| v.as_f64().map(|f| f as f32)).collect();
    vector.map(Some).ok_or_else(|| {
        Error::Query(format!(
            "similarity() field '{field}' contains non-numeric values."
        ))
    })
}

/// Whether two `similarity()` sit on both sides of one `OR`.
fn has_multiple_similarity_in_or(condition: &Condition) -> bool {
    any_subtree(condition, &|c| {
        matches!(c, Condition::Or(left, right)
            if count_matching_leaves(left, is_vector_leaf) > 0
                && count_matching_leaves(right, is_vector_leaf) > 0
        )
    })
}

/// Rejects a `NEAR_FUSED` the fusion executor cannot run as written.
fn reject_unexecutable_fused_search(condition: &Condition) -> Result<()> {
    // `vector_or_sparse_count` includes SPARSE_NEAR leaves (which
    // `is_vector_leaf` omits) so "NEAR_FUSED AND SPARSE_NEAR" is rejected
    // just like "NEAR_FUSED AND NEAR / similarity()".
    let fused_count =
        count_matching_leaves(condition, |c| matches!(c, Condition::VectorFusedSearch(_)));
    let vector_or_sparse_count = count_matching_leaves(condition, |c| {
        is_vector_leaf(c) || matches!(c, Condition::SparseVectorSearch(_))
    });
    if fused_count > 0
        && (fused_count > 1
            || vector_or_sparse_count > fused_count
            || fused_under_or_or_not(condition))
    {
        return Err(Error::Query(
            "NEAR_FUSED must be the only vector predicate and cannot appear under OR/NOT; \
             combine it only with AND <metadata filter>."
                .to_string(),
        ));
    }
    Ok(())
}

/// Rejects a condition with more than one `SPARSE_NEAR`: a query runs a
/// single sparse branch, so a second one would be dropped without a word.
/// And one under `NOT`: the sparse search would rank the very rows the
/// negation excludes — a dense `NEAR` under `NOT` reads TRUE and takes the
/// full scan, selecting none — so the negation would be dropped instead.
fn reject_unexecutable_sparse_search(condition: &Condition) -> Result<()> {
    let sparse_count = count_matching_leaves(condition, is_sparse_search);
    if sparse_count > 1 {
        return Err(Error::Query(format!(
            "the query has {sparse_count} SPARSE_NEAR clauses, and a query runs one \
             sparse search; run one query per sparse vector and fuse the results"
        )));
    }
    if any_subtree(
        condition,
        &|c| matches!(c, Condition::Not(inner) if count_matching_leaves(inner, is_sparse_search) > 0),
    ) {
        return Err(Error::Query(
            "a SPARSE_NEAR cannot appear under NOT: it ranks the rows it matches; \
             negate the metadata conditions beside it instead"
                .to_string(),
        ));
    }
    Ok(())
}

const fn is_sparse_search(condition: &Condition) -> bool {
    matches!(condition, Condition::SparseVectorSearch(_))
}

/// Rejects a text `MATCH` that would rank beside a ranking no executor
/// fuses it with (`super::text_match`).
///
/// A `MATCH` the WHERE AND-requires ranks the rows: alone, by BM25; beside a
/// dense `NEAR`, as the text branch of a hybrid search. Every other `MATCH` —
/// under `OR` or `NOT`, or beside a `similarity()` threshold, which ranks no
/// rows — is a membership test any executor evaluates. Refused:
///
/// - an AND-required `MATCH` beside a `SPARSE_NEAR` or a `NEAR_FUSED`, whose
///   executors fuse no text branch, or beside a `NEAR` and a `similarity()`
///   threshold, which no hybrid search runs;
/// - two AND-required `MATCH` where one would rank (no `similarity()`
///   threshold beside them): a query runs one text search.
fn reject_unexecutable_text_match(condition: &Condition) -> Result<()> {
    let mut ranked = Vec::new();
    and_required_text_matches(condition, &mut ranked);
    if ranked.is_empty() {
        return Ok(());
    }
    let holds = |leaf: fn(&Condition) -> bool| count_matching_leaves(condition, leaf) > 0;
    let similarity = holds(|c| matches!(c, Condition::Similarity(_)));
    if holds(|c| {
        matches!(
            c,
            Condition::SparseVectorSearch(_) | Condition::VectorFusedSearch(_)
        )
    }) || (similarity && ranking_near(condition))
    {
        return Err(Error::Query(
            "a text MATCH AND-ed with the WHERE is fused with its ranking, and only a \
             vector NEAR without a similarity() threshold is fused with text, not \
             SPARSE_NEAR or NEAR_FUSED; run the text search as its own query, or test \
             the MATCH under OR / NOT"
                .to_string(),
        ));
    }
    if ranked.len() > 1 && !similarity {
        return Err(Error::Query(format!(
            "the query ANDs {} text MATCH clauses, and a query ranks by one text search; \
             combine the terms in one MATCH, or run one query per MATCH",
            ranked.len()
        )));
    }
    Ok(())
}

/// True if any `NEAR_FUSED` leaf sits under an `OR` or `NOT`, where the fusion
/// executor (which only walks AND/Group) would not reach it.
fn fused_under_or_or_not(condition: &Condition) -> bool {
    fn has_fused(c: &Condition) -> bool {
        count_matching_leaves(c, |x| matches!(x, Condition::VectorFusedSearch(_))) > 0
    }
    any_subtree(condition, &|c| match c {
        Condition::Or(l, r) => has_fused(l) || has_fused(r),
        Condition::Not(inner) => has_fused(inner),
        _ => false,
    })
}

#[cfg(test)]
#[path = "vector_predicates_tests.rs"]
mod tests;
