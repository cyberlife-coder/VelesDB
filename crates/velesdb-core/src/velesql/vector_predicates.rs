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
/// - more than one `SPARSE_NEAR` (a query runs one sparse branch);
/// - a `NEAR_FUSED` that is not the only vector predicate, or that sits
///   under `OR` / `NOT` (the fusion executor would not reach it).
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
    reject_more_than_one_sparse_search(condition)?;
    reject_unexecutable_fused_search(condition)
}

/// The vector the payload holds under `field`, for a named-vector
/// `similarity(field, $v)`: `None` when the point has no such field (it
/// simply has no vector for it).
///
/// # Errors
///
/// [`Error::Query`] when the field holds something other than an array of
/// numbers.
pub fn payload_vector(payload: &serde_json::Value, field: &str) -> Result<Option<Vec<f32>>> {
    let Some(value) = payload.get(field) else {
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
fn reject_more_than_one_sparse_search(condition: &Condition) -> Result<()> {
    let sparse_count =
        count_matching_leaves(condition, |c| matches!(c, Condition::SparseVectorSearch(_)));
    if sparse_count > 1 {
        return Err(Error::Query(format!(
            "the query has {sparse_count} SPARSE_NEAR clauses, and a query runs one \
             sparse search; run one query per sparse vector and fuse the results"
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
