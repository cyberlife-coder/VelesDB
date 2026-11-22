use super::*;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::Parser;

#[test]
fn a_requested_full_window_and_a_truncated_window_are_distinct() {
    let max = velesdb_core::velesql::MAX_SELECT_LIMIT;
    let search = |tail: &str| {
        let sql = format!("SELECT * FROM c WHERE vector SPARSE_NEAR {{7: 1.0}} {tail}");
        CandidateWindow::for_search(&Parser::parse(&sql).expect("test: parse").select, true)
    };
    assert_eq!(
        search(&format!("ORDER BY similarity() DESC LIMIT {max}")),
        CandidateWindow::Requested(usize::try_from(max).expect("test: ceiling"))
    );
    assert_eq!(
        search(&format!("ORDER BY similarity() DESC LIMIT 1 OFFSET {max}")),
        CandidateWindow::Capped
    );
    assert_eq!(search("ORDER BY id DESC LIMIT 1"), CandidateWindow::Capped);
}

fn seed(db: &mut DatabaseInner) {
    db.create_collection("vecs", 4, "cosine")
        .expect("test: create");
    let store = db.get_shared_store("vecs").expect("test: store");
    for (id, v, cat) in [
        (10u64, vec![1.0, 0.0, 0.0, 0.0], "a"),
        (11, vec![0.9, 0.1, 0.0, 0.0], "a"),
        (12, vec![0.0, 1.0, 0.0, 0.0], "b"),
        (13, vec![0.0, 0.0, 1.0, 0.0], "b"),
    ] {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &v,
            Some(serde_json::json!({ "cat": cat })),
        );
    }
}

fn run(db: &DatabaseInner, sql: &str, params_json: &str) -> Vec<OwnedScanRow> {
    let q = Parser::parse(sql).expect("test: parse");
    let params = parse_params(Some(params_json)).expect("test: params");
    let fused = find_fused_search(q.select.where_clause.as_ref()).expect("test: fused");
    execute_fused_search(db, &q.select, fused, &params).expect("test: fused exec")
}

#[test]
fn test_fused_returns_fused_ranking() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    // Both query vectors point at id 12 ([0,1,0,0]): $a is exactly it and
    // $b is dominated by the y-axis. So id 12 is the unambiguous top of the
    // fused ranking even though it is stored THIRD — a no-op that returned
    // storage order (10,11,12,13), the bug class this path exists to kill,
    // would put 10 first and fail the assertion below.
    let rows = run(
        &db,
        "SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b]",
        r#"{"a": [0.0, 1.0, 0.0, 0.0], "b": [0.1, 0.9, 0.0, 0.0]}"#,
    );
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    assert_eq!(ids.len(), 4, "all four ids present in the fused ranking");
    assert_eq!(
        ids[0], 12,
        "id 12 (favored by both query vectors) must be the top fused result, not storage-order id 10"
    );
    // Fused output is ordered by descending fused score (a real ranking).
    let scores: Vec<f32> = rows.iter().map(|r| r.score).collect();
    assert!(
        scores.windows(2).all(|w| w[0] >= w[1]),
        "rows must be sorted by descending fused score, got {scores:?}"
    );
}

#[test]
fn test_fused_with_metadata_filter() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let rows = run(
        &db,
        "SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] AND cat = 'a'",
        r#"{"a": [1.0, 0.0, 0.0, 0.0], "b": [0.0, 1.0, 0.0, 0.0]}"#,
    );
    let ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    assert_eq!(
        ids.len(),
        2,
        "only cat='a' rows survive the pre-fusion filter"
    );
    assert!(ids.contains(&10) && ids.contains(&11));
}

/// The isolation contract is the one core enforces, applied before any scan
/// (`velesdb_core::velesql::validate_vector_predicates`).
fn refusal(sql: &str) -> String {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    crate::velesql_exec::execute(
        &mut db,
        sql,
        Some(
            r#"{"a": [1.0, 0.0, 0.0, 0.0], "b": [0.0, 1.0, 0.0, 0.0], "c": [0.0, 0.0, 1.0, 0.0]}"#,
        ),
    )
    .map(|_| ())
    .expect_err("test: refused")
    .to_string()
}

#[test]
fn test_fused_rejects_under_or() {
    let err = refusal("SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] OR cat = 'b'");
    assert!(err.contains("NEAR_FUSED"), "{err}");
}

#[test]
fn test_fused_rejects_mixed_with_near() {
    let err = refusal("SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] AND vector NEAR $c");
    assert!(err.contains("NEAR_FUSED"), "{err}");
}

/// The `count` an aggregate SELECT reports.
fn count(db: &mut DatabaseInner, sql: &str) -> u64 {
    let result = crate::velesql_exec::execute(
        db,
        sql,
        Some(r#"{"a": [1.0, 0.0, 0.0, 0.0], "b": [0.0, 1.0, 0.0, 0.0]}"#),
    )
    .expect("test: aggregate");
    let rows: serde_json::Value =
        serde_json::from_str(&result.rows_json()).expect("test: rows json");
    rows[0]["count"].as_u64().expect("test: count")
}

/// An aggregate reads the whole collection, as core's `execute_aggregate`
/// does: a `NEAR_FUSED` leaf filters no point there, so neither the output
/// LIMIT nor the default window may bound what COUNT sees.
#[test]
fn test_fused_aggregate_counts_every_point_whatever_the_limit() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let store = db.get_shared_store("vecs").expect("test: store");
    for id in 100u64..130 {
        crate::store_insert::insert_with_payload(
            &mut store.borrow_mut(),
            id,
            &[0.5, 0.5, 0.5, 0.5],
            Some(serde_json::json!({ "cat": "c" })),
        );
    }
    let fused = "SELECT COUNT(*) FROM vecs WHERE vector NEAR_FUSED [$a, $b]";
    assert_eq!(count(&mut db, fused), 34);
    assert_eq!(count(&mut db, &format!("{fused} LIMIT 1")), 34);
    assert_eq!(
        count(&mut db, &format!("{fused} AND cat = 'c' LIMIT 1")),
        30
    );
}
