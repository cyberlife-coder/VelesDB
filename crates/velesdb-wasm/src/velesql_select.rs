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

use std::collections::HashMap;

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
        // Finding H: reject WHERE clauses that mix similarity() predicates
        // against distinct query vectors — SimilarityEvaluator pre-computes
        // scores for a single vector and would otherwise silently return
        // wrong rows for the second threshold.
        velesql_similarity::assert_single_similarity_vector(normalized.as_ref())?;
        // Finding P: `normalized` is already push_not_inward-ed above;
        // use the pre-normalized variants to skip redundant clones + tree
        // walks inside `find_similarity` / `strip_similarity`.
        let similarity_cond =
            velesql_similarity::find_similarity_pre_normalized(normalized.as_ref());
        let residual = velesql_similarity::strip_similarity_pre_normalized(normalized.as_ref());
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

    /// Returns `true` if this filter set has no `similarity()` predicate.
    ///
    /// Used by the fusion path to short-circuit `collect_vector_rows`'
    /// post-filter: when fusion has already applied the residual WHERE
    /// via `fusion_branch_from_residual` AND there is no similarity leaf
    /// to re-evaluate, the post-filter is fully redundant and can be
    /// skipped (Devin Review Finding Q). The non-fusion path must still
    /// apply the post-filter — this accessor is only consulted under a
    /// fusion-active branch.
    fn has_no_similarity(&self) -> bool {
        self.eval.is_none()
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

    // Finding Q: when fusion is active AND the filter set has no
    // similarity predicate to re-evaluate row-wise, the residual WHERE
    // has already been applied by `fusion_branch_from_residual` — the
    // post-filter in `collect_vector_rows` would duplicate that work.
    // We skip it by passing `None` for the filter set in that case.
    // The non-fusion path (and fusion + similarity) still applies
    // `filters.passes()` so correctness is preserved.
    let mut fusion_residual_already_applied = false;
    if let Some(clause) = &stmt.fusion_clause {
        scored = apply_fusion(
            &scored,
            clause,
            &borrowed,
            filters.residual.as_ref(),
            params,
        )?;
        fusion_residual_already_applied = filters.has_no_similarity();
    }

    if fusion_residual_already_applied {
        collect_vector_rows_unfiltered(&scored, &borrowed)
    } else {
        collect_vector_rows(&scored, &borrowed, &filters, params)
    }
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

/// Builds an `id -> index` lookup map so the vector-row collectors can
/// resolve ids in O(1) instead of O(n) per row (Devin Review Finding F10).
///
/// The non-fusion path walks every scored row, calls this once per query,
/// and then hits the map N times; overall work goes from O(n^2) to O(n).
fn build_id_to_idx(store: &crate::vector_store::VectorStore) -> HashMap<u64, usize> {
    store
        .ids
        .iter()
        .enumerate()
        .map(|(i, &id)| (id, i))
        .collect()
}

/// Fusion-path collector: returns every scored row verbatim, skipping
/// the residual WHERE re-check because [`fusion_branch_from_residual`]
/// already applied it (Devin Review Finding Q). Only used when there is
/// no similarity predicate to re-evaluate row-wise.
fn collect_vector_rows_unfiltered(
    scored: &[(u64, f32)],
    store: &crate::vector_store::VectorStore,
) -> Result<Vec<OwnedScanRow>, String> {
    // Finding F10: hash-map lookup, O(1) per row, O(n) build cost paid
    // once. Previous `store.ids.iter().position(...)` was O(n) per row,
    // i.e. O(n^2) overall — catastrophic on 10k+ row collections.
    let id_to_idx = build_id_to_idx(store);
    let mut out = Vec::with_capacity(scored.len());
    for &(id, score) in scored {
        // INVARIANT: fused scores come from the union of the vector
        // branch (which only yields known ids) and the payload branch
        // (ids from `store.ids`). If the id isn't in the store, drop it
        // rather than panicking — fusion strategies like RRF may pair
        // the same id from both branches, but never invent new ids.
        let Some(&idx) = id_to_idx.get(&id) else {
            continue;
        };
        let payload = store.payloads.get(idx).and_then(|p| p.as_ref());
        out.push((id, score, payload.cloned()));
    }
    Ok(out)
}

fn collect_vector_rows(
    scored: &[(u64, f32)],
    store: &crate::vector_store::VectorStore,
    filters: &WhereFilters,
    params: &Params,
) -> Result<Vec<OwnedScanRow>, String> {
    // Finding F10: same O(n) → O(1) lookup as
    // `collect_vector_rows_unfiltered` but on the filtered path. The
    // map is built once before the loop, so the overall cost is O(n)
    // instead of O(n^2) (previous `.position(...)` walked store.ids
    // for every scored row).
    let id_to_idx = build_id_to_idx(store);
    let mut out = Vec::with_capacity(scored.len());
    for &(id, score) in scored {
        // INVARIANT: `compute_scores` only yields ids that were in
        // `store.ids` at scoring time, so this `get` always finds the
        // row. We propagate an explicit error rather than `.expect()`
        // (Devin Review Finding N) to avoid a panic in production paths
        // if some future refactor breaks the invariant.
        let idx = *id_to_idx
            .get(&id)
            .ok_or_else(|| format!("internal: compute_scores yielded unknown id {id}"))?;
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
    // INVARIANT: `QueryResultRow::build` only fails on serde_json encoding
    // errors, which cannot occur here (all inputs are typed primitives and
    // already-validated JSON payloads). We still propagate the error via
    // `?` rather than `.expect()` (Devin Review Finding N) so a future
    // change to the payload representation fails cleanly.
    let mut sortable: Vec<SortableRow> = rows
        .into_iter()
        .map(|(id, score, payload)| {
            QueryResultRow::build(id, score, payload.as_ref()).map(|row| SortableRow {
                id,
                score,
                payload,
                row,
            })
        })
        .collect::<Result<_, _>>()?;
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
///
/// The input is first passed through [`crate::velesql_logic::push_not_inward`]
/// so NOT is De-Morgan-distributed before stripping — symmetric with
/// [`velesql_similarity::strip_similarity`]. Without this normalization, a
/// query like `NOT (vector NEAR $q OR cat = 'a')` would recurse into the
/// NOT wrapper, collapse the stripped VectorSearch under OR to `None`
/// (via `combine_after_strip`, finding G), then wrap `None` back up
/// through Not → None, leaving `WhereFilters.normalized` empty and
/// `passes()` true for every row. Instead, `push_not_inward` rewrites to
/// `NOT VectorSearch AND NOT cat='a'`; the strip pass removes only the
/// bare VectorSearch leaf (logically `true` via the external NEAR path),
/// and `true AND NOT cat='a'` correctly reduces to `NOT cat='a'`.
fn strip_vector_search(cond: Option<&Condition>) -> Option<Condition> {
    let normalized = cond.cloned().map(crate::velesql_logic::push_not_inward);
    velesql_similarity::strip_condition_if(normalized.as_ref(), &|c| {
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

    // --- Finding F10: O(n) id lookup scales linearly, not quadratically ---
    //
    // Regression test: with 500 rows, the previous O(n^2) `.position(...)`
    // path did 500 * 500 = 250_000 id comparisons per NEAR query. The
    // hash-map path does ~500 lookups. This test asserts correctness at a
    // scale large enough that a future regression to `.position(...)` would
    // visibly slow the suite; the numbers themselves are not the point —
    // correctness is.

    #[test]
    fn test_select_near_scales_with_hashmap_lookup() {
        let mut db = DatabaseInner::new();
        db.create_collection("vecs_large", 4, "cosine")
            .expect("test: create");
        let store = db.get_shared_store("vecs_large").expect("test: store");
        for i in 0u64..500 {
            #[allow(clippy::cast_precision_loss)]
            let val = i as f32;
            crate::store_insert::insert_with_payload(
                &mut store.borrow_mut(),
                i,
                &[val, 0.0, 0.0, 0.0],
                None,
            );
        }
        drop(store);
        let q = parse_query("SELECT * FROM vecs_large WHERE vector NEAR $q LIMIT 10");
        let params = parse_params(Some(r#"{"q": [1.0, 0.0, 0.0, 0.0]}"#)).expect("test: p");
        let rows = execute(&mut db, &q, &params).expect("test: near");
        // With cosine, only rows with strictly positive first component
        // match; row id=0 has a zero-norm vector and returns NaN score
        // (filtered out). We assert we got 10 rows (LIMIT) and that they
        // are all from the seeded collection (id < 500).
        assert_eq!(rows.len(), 10);
        for row in &rows {
            assert!(row.id() < 500, "id should come from seeded range");
        }
    }
}
