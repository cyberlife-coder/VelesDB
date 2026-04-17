//! SELECT dispatch for the WASM VelesQL executor (S4-13).
//!
//! Orchestrates the SELECT pipeline:
//!
//! 1. **Vector-search branch**: if WHERE contains `vector NEAR $v`, run
//!    brute-force scoring, then apply any non-vector / non-similarity
//!    WHERE predicates as a post-filter.
//! 2. **Similarity-threshold branch**: if WHERE contains `similarity(v, $q)
//!    <op> t`, compute similarities inline and use them as a filter.
//! 3. **JOIN branch**: if `stmt.joins` is non-empty, hand off to
//!    [`crate::velesql_join`] which does a nested-loop join.
//! 4. **Plain-scan branch**: otherwise, walk the collection and apply WHERE.
//!
//! Post-scan, the pipeline honors DISTINCT, GROUP BY / HAVING, ORDER BY,
//! LIMIT / OFFSET, and FUSION clauses. Each of these features lives in a
//! dedicated sibling module so this file stays a pure orchestrator.

use velesdb_core::velesql::{Condition, Query, SelectStatement, VectorSearch};

use crate::database::DatabaseInner;
use crate::vector_ops;
use crate::velesql_aggregate;
use crate::velesql_fusion;
use crate::velesql_orderby::{self, SortableRow};
use crate::velesql_result::QueryResultRow;
use crate::velesql_scan::OwnedScanRow;
use crate::velesql_similarity::{self, SimilarityEvaluator};
use crate::velesql_value::{resolve_vector, Params};
use crate::velesql_where;

/// Executes a SELECT query and returns its row set.
pub(crate) fn execute(
    db: &mut DatabaseInner,
    query: &Query,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    if !query.let_bindings.is_empty() {
        return Err("LET bindings are not supported in WASM".to_string());
    }
    if !query.select.joins.is_empty() {
        return crate::velesql_join::execute(db, &query.select, params);
    }

    let rows = if let Some(vs) = find_vector_search(query.select.where_clause.as_ref()) {
        execute_vector_search(db, &query.select, vs, params)?
    } else {
        execute_plain(db, &query.select, params)?
    };

    finalize(&query.select, rows, params)
}

// --- Plain scan ----------------------------------------------------------

/// Runs a non-vector SELECT (optionally with a similarity() threshold in
/// the WHERE clause).
fn execute_plain(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    let filters = WhereFilters::build(db, stmt, params, None)?;
    let store = db.get_shared_store(&stmt.from)?;
    let borrowed = store.borrow();
    let mut out = Vec::with_capacity(borrowed.ids.len());
    for (idx, &id) in borrowed.ids.iter().enumerate() {
        let payload = borrowed.payloads.get(idx).and_then(|p| p.as_ref());
        if !filters.passes(id, payload, idx, &borrowed, params)? {
            continue;
        }
        out.push((id, 0.0, payload.cloned()));
    }
    Ok(out)
}

/// Bundles the payload + similarity filters applied row-wise by the SELECT
/// pipeline. Keeps `execute_plain` and `execute_vector_search` readable
/// without duplicating the row-check boilerplate.
///
/// Two condition views are kept:
///
/// - `normalized`: the full WHERE clause after [`push_not_inward`]. Still
///   contains `Similarity(_)` leaves; evaluated row-wise via
///   [`velesql_similarity::evaluate_where_with_similarity`] so boolean
///   composition (AND / OR / NOT) around similarity stays correct —
///   notably for De-Morgan-rewritten compounds like
///   `NOT (sim > t AND x = 1)` ⇒ `sim <= t OR x != 1`.
/// - `residual`: the normalized clause with every `Similarity(_)` leaf
///   stripped out. Used by the fusion path's "payload branch" where we
///   only want rows matching the non-vector, non-similarity predicates.
struct WhereFilters {
    normalized: Option<Condition>,
    residual: Option<Condition>,
    eval: Option<SimilarityEvaluator>,
}

impl WhereFilters {
    /// Builds the row filter set for a SELECT. The plain path leaves
    /// `pre_stripped_base` as None (meaning "use `stmt.where_clause`"); the
    /// vector path wraps the already-stripped condition in `Some(...)` so
    /// "vector NEAR $v" alone produces `Some(None)` — a valid "no residual"
    /// state that must NOT fall back to the raw WHERE clause.
    fn build(
        db: &DatabaseInner,
        stmt: &SelectStatement,
        params: &Params,
        pre_stripped_base: Option<Option<Condition>>,
    ) -> Result<Self, String> {
        let base = match pre_stripped_base {
            Some(already_stripped) => already_stripped,
            None => stmt.where_clause.clone(),
        };
        let normalized = base.map(crate::velesql_logic::push_not_inward);
        let similarity_cond = velesql_similarity::find_similarity(normalized.as_ref());
        let residual = velesql_similarity::strip_similarity(normalized.as_ref());
        let eval = similarity_cond
            .as_ref()
            .map(|c| SimilarityEvaluator::new(db, &stmt.from, c, params))
            .transpose()?;
        Ok(Self {
            normalized,
            residual,
            eval,
        })
    }

    fn passes(
        &self,
        id: u64,
        payload: Option<&serde_json::Value>,
        idx: usize,
        _store: &crate::vector_store::VectorStore,
        params: &Params,
    ) -> Result<bool, String> {
        let Some(cond) = self.normalized.as_ref() else {
            return Ok(true);
        };
        velesql_similarity::evaluate_where_with_similarity(
            cond,
            id,
            payload,
            idx,
            self.eval.as_ref(),
            params,
        )
    }
}

// --- Vector NEAR ---------------------------------------------------------

fn execute_vector_search(
    db: &DatabaseInner,
    stmt: &SelectStatement,
    vector_search: &VectorSearch,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    let store = db.get_shared_store(&stmt.from)?;
    let borrowed = store.borrow();
    validate_vector_collection(&borrowed, &stmt.from)?;

    let query_vec = resolve_query_vector(vector_search, &borrowed, params)?;
    let mut scored = score_all(&query_vec, &borrowed);

    let residual_without_vector = strip_vector_search(stmt.where_clause.as_ref());
    let filters = WhereFilters::build(db, stmt, params, Some(residual_without_vector.clone()))?;

    if let Some(clause) = &stmt.fusion_clause {
        scored = apply_fusion(
            &scored,
            clause,
            &borrowed,
            filters.residual.as_ref(),
            params,
        )?;
    }

    collect_vector_rows(&scored, &borrowed, &filters, params)
}

fn validate_vector_collection(
    store: &crate::vector_store::VectorStore,
    name: &str,
) -> Result<(), String> {
    if store.dimension == 0 {
        return Err(format!(
            "Collection '{name}' is metadata-only; NEAR queries require a vector collection"
        ));
    }
    Ok(())
}

fn resolve_query_vector(
    vs: &VectorSearch,
    store: &crate::vector_store::VectorStore,
    params: &Params,
) -> Result<Vec<f32>, String> {
    let q = resolve_vector(&vs.vector, params)?;
    if q.len() != store.dimension {
        return Err(format!(
            "NEAR query dimension mismatch: expected {}, got {}",
            store.dimension,
            q.len()
        ));
    }
    Ok(q)
}

fn score_all(query: &[f32], store: &crate::vector_store::VectorStore) -> Vec<(u64, f32)> {
    let mut scored = vector_ops::compute_scores(
        query,
        &store.ids,
        &store.data,
        &store.data_sq8,
        &store.data_binary,
        &store.sq8_mins,
        &store.sq8_scales,
        store.dimension,
        store.metric,
        store.storage_mode,
    );
    vector_ops::sort_results(&mut scored, store.metric.higher_is_better());
    scored
}

fn apply_fusion(
    scored: &[(u64, f32)],
    clause: &velesdb_core::velesql::FusionClause,
    store: &crate::vector_store::VectorStore,
    residual: Option<&Condition>,
    params: &Params,
) -> Result<Vec<(u64, f32)>, String> {
    let rrf_branch = scored.to_vec();
    let payload_branch = fusion_branch_from_residual(store, residual, params)?;
    Ok(velesql_fusion::apply(
        clause,
        vec![rrf_branch, payload_branch],
    ))
}

fn collect_vector_rows(
    scored: &[(u64, f32)],
    store: &crate::vector_store::VectorStore,
    filters: &WhereFilters,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    let mut out = Vec::with_capacity(scored.len());
    for &(id, score) in scored {
        let idx = store
            .ids
            .iter()
            .position(|&x| x == id)
            .expect("compute_scores only yields known ids");
        let payload = store.payloads.get(idx).and_then(|p| p.as_ref());
        if !filters.passes(id, payload, idx, store, params)? {
            continue;
        }
        out.push((id, score, payload.cloned()));
    }
    Ok(out)
}

fn fusion_branch_from_residual(
    store: &crate::vector_store::VectorStore,
    cond: Option<&Condition>,
    params: &Params,
) -> Result<Vec<(u64, f32)>, String> {
    let Some(cond) = cond else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (idx, &id) in store.ids.iter().enumerate() {
        let payload = store.payloads.get(idx).and_then(|p| p.as_ref());
        if velesql_where::matches(cond, id, payload, params)? {
            out.push((id, 1.0));
        }
    }
    Ok(out)
}

// --- Finalization --------------------------------------------------------

/// Applies DISTINCT / GROUP BY / HAVING / ORDER BY / LIMIT / OFFSET once the
/// raw row set has been materialised by either the plain or vector path.
fn finalize(
    stmt: &SelectStatement,
    rows: Vec<OwnedScanRow>,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    if velesql_aggregate::needs_aggregation_pipeline(stmt) {
        return finalize_aggregated(stmt, rows, params);
    }
    finalize_plain(stmt, rows)
}

fn finalize_aggregated(
    stmt: &SelectStatement,
    rows: Vec<OwnedScanRow>,
    params: &Params,
) -> Result<Vec<QueryResultRow>, String> {
    let scanned: Vec<_> = rows
        .iter()
        .map(|(id, score, p)| (*id, *score, p.as_ref()))
        .collect();
    let out = velesql_aggregate::apply(stmt, &scanned, params)?;
    Ok(apply_limit_offset(stmt, out))
}

fn finalize_plain(
    stmt: &SelectStatement,
    rows: Vec<OwnedScanRow>,
) -> Result<Vec<QueryResultRow>, String> {
    let mut sortable: Vec<SortableRow> = rows
        .into_iter()
        .map(|(id, score, payload)| {
            let row = QueryResultRow::build(id, score, payload.as_ref())
                .expect("row serialization never fails with typed inputs");
            SortableRow {
                id,
                score,
                payload,
                row,
            }
        })
        .collect();
    velesql_orderby::sort_rows(stmt, &mut sortable);
    let rows: Vec<QueryResultRow> = sortable.into_iter().map(|s| s.row).collect();
    Ok(apply_limit_offset(stmt, rows))
}

fn apply_limit_offset(stmt: &SelectStatement, rows: Vec<QueryResultRow>) -> Vec<QueryResultRow> {
    let offset = usize::try_from(stmt.offset.unwrap_or(0)).unwrap_or(usize::MAX);
    let limit = usize::try_from(stmt.limit.unwrap_or(u64::MAX)).unwrap_or(usize::MAX);
    rows.into_iter().skip(offset).take(limit).collect()
}

// --- Condition-tree helpers ---------------------------------------------

fn find_vector_search(cond: Option<&Condition>) -> Option<&VectorSearch> {
    let cond = cond?;
    match cond {
        Condition::VectorSearch(vs) => Some(vs),
        Condition::And(l, r) | Condition::Or(l, r) => {
            find_vector_search(Some(l)).or_else(|| find_vector_search(Some(r)))
        }
        Condition::Not(inner) | Condition::Group(inner) => find_vector_search(Some(inner)),
        _ => None,
    }
}

/// Returns the condition with every `VectorSearch` / `VectorFusedSearch` /
/// `SparseVectorSearch` subtree removed.
fn strip_vector_search(cond: Option<&Condition>) -> Option<Condition> {
    velesql_similarity::strip_condition_if(cond, &|c| {
        matches!(
            c,
            Condition::VectorSearch(_)
                | Condition::VectorFusedSearch(_)
                | Condition::SparseVectorSearch(_)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::DatabaseInner;
    use crate::velesql_value::parse_params;
    use velesdb_core::velesql::Parser;

    fn parse_query(sql: &str) -> Query {
        Parser::parse(sql).expect("test: parse")
    }

    fn seed_metadata_docs(db: &mut DatabaseInner) {
        db.create_metadata_collection("docs").expect("test: create");
        let store = db.get_shared_store("docs").expect("test: store");
        let mut borrowed = store.borrow_mut();
        for (id, cat) in [(1u64, "tech"), (2, "food"), (3, "tech")] {
            borrowed.ids.push(id);
            borrowed
                .payloads
                .push(Some(serde_json::json!({"cat": cat})));
        }
    }

    fn seed_vector_collection(db: &mut DatabaseInner) {
        db.create_collection("vecs", 4, "cosine")
            .expect("test: create");
        let store = db.get_shared_store("vecs").expect("test: store");
        for (id, v) in [
            (10u64, vec![1.0, 0.0, 0.0, 0.0]),
            (11, vec![0.0, 1.0, 0.0, 0.0]),
            (12, vec![0.0, 0.0, 1.0, 0.0]),
        ] {
            crate::store_insert::insert_with_payload(
                &mut store.borrow_mut(),
                id,
                &v,
                Some(serde_json::json!({"cat": if id == 10 { "a" } else { "b" }})),
            );
        }
    }

    #[test]
    fn test_select_all_returns_all_rows() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let q = parse_query("SELECT * FROM docs");
        let rows =
            execute(&mut db, &q, &parse_params(None).expect("test: p")).expect("test: select");
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn test_select_with_limit() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let q = parse_query("SELECT * FROM docs LIMIT 2");
        let rows =
            execute(&mut db, &q, &parse_params(None).expect("test: p")).expect("test: select");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_select_where_filters() {
        let mut db = DatabaseInner::new();
        seed_metadata_docs(&mut db);
        let q = parse_query("SELECT * FROM docs WHERE cat = 'tech'");
        let rows =
            execute(&mut db, &q, &parse_params(None).expect("test: p")).expect("test: select");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn test_select_near_returns_ranked_results() {
        let mut db = DatabaseInner::new();
        seed_vector_collection(&mut db);
        let q = parse_query("SELECT * FROM vecs WHERE vector NEAR $q LIMIT 2");
        let params = parse_params(Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#)).expect("test: p");
        let rows = execute(&mut db, &q, &params).expect("test: near");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id(), 10);
    }

    #[test]
    fn test_select_near_dimension_mismatch_errors() {
        let mut db = DatabaseInner::new();
        seed_vector_collection(&mut db);
        let q = parse_query("SELECT * FROM vecs WHERE vector NEAR $q LIMIT 2");
        let params = parse_params(Some(r#"{"q": [1.0, 0.0]}"#)).expect("test: p");
        let err = execute(&mut db, &q, &params);
        assert!(err.is_err());
    }
}
