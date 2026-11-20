//! Shared row-scan helpers for the WASM SELECT pipeline (S4-13).
//!
//! Extracts the WHERE-filtered row iteration that both the plain-scan path
//! (payload-only SELECT) and the aggregation pipeline need. Keeps
//! `velesql_select.rs` under the NLOC limit while giving the aggregator a
//! clean API to receive pre-filtered rows without re-implementing the walk.

use velesdb_core::velesql::Condition;

use crate::database::DatabaseInner;
use crate::exec_error::ExecError;
use crate::velesql_value::Params;
use crate::velesql_where;

/// A single scanned row owned by the caller.
///
/// Owning the payload (rather than borrowing) lets the caller drop the
/// store lock before running the aggregation / sort pipeline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedScanRow {
    pub(crate) id: u64,
    pub(crate) score: f32,
    /// Whether `score` is the row's vector similarity — a `NEAR`'s, or the
    /// first `similarity()` leaf's when it could score the row. Such a row
    /// carries its score as its `vector_score` component, as core tags it
    /// (`tag_vector_component_scores`); a metadata match, a fused or a sparse
    /// score carries no component breakdown.
    pub(crate) vector_scored: bool,
    pub(crate) payload: Option<serde_json::Value>,
}

impl OwnedScanRow {
    /// A row whose score is not a vector similarity.
    pub(crate) fn new(id: u64, score: f32, payload: Option<serde_json::Value>) -> Self {
        Self {
            id,
            score,
            vector_scored: false,
            payload,
        }
    }

    /// A row whose score is its vector similarity.
    pub(crate) fn vector_scored(id: u64, score: f32, payload: Option<serde_json::Value>) -> Self {
        Self {
            vector_scored: true,
            ..Self::new(id, score, payload)
        }
    }
}

/// Scans a metadata-only or vector collection end-to-end, applying the
/// optional WHERE clause. Returns rows in insertion order with `score` set
/// to 0.0 (vector search happens in `velesql_select::vector_path`).
pub(crate) fn scan_all(
    db: &DatabaseInner,
    from: &str,
    where_clause: Option<&Condition>,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, ExecError> {
    let store = db.get_shared_store(from)?;
    let borrowed = store.borrow();
    // Capacity is bounded by the guardrail refusal, never by the raw store
    // size: `with_capacity(ids.len())` preallocated the whole dataset even
    // when the WHERE matched a handful of rows (#2679).
    let mut out = Vec::with_capacity(
        borrowed
            .ids
            .len()
            .min(crate::guardrails::MAX_INTERMEDIATE_CARDINALITY),
    );
    let filter = where_clause
        .map(|cond| velesql_where::WhereFilter::compile(cond, params))
        .transpose()?;
    for (idx, &id) in borrowed.ids.iter().enumerate() {
        let payload = borrowed.payloads.get(idx).and_then(|p| p.as_ref());
        if filter.as_ref().is_some_and(|f| !f.matches(id, payload)) {
            continue;
        }
        out.push(OwnedScanRow::new(id, 0.0, payload.cloned()));
        crate::guardrails::check_cardinality(out.len())?;
    }
    Ok(out)
}

#[cfg(test)]
#[path = "velesql_scan_tests.rs"]
mod tests;
