use super::*;
use crate::velesql_value::parse_params;
use velesdb_core::velesql::Parser;

fn parse_where(sql: &str) -> Option<Condition> {
    Parser::parse(sql).expect("test: parse").select.where_clause
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
    let ids: Vec<u64> = rows.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(ids.len(), 4, "all four ids present in the fused ranking");
    assert_eq!(
        ids[0], 12,
        "id 12 (favored by both query vectors) must be the top fused result, not storage-order id 10"
    );
    // Fused output is ordered by descending fused score (a real ranking).
    let scores: Vec<f32> = rows.iter().map(|(_, s, _)| *s).collect();
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
    let ids: Vec<u64> = rows.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(
        ids.len(),
        2,
        "only cat='a' rows survive the pre-fusion filter"
    );
    assert!(ids.contains(&10) && ids.contains(&11));
}

#[test]
fn test_fused_rejects_under_or() {
    let mut db = DatabaseInner::new();
    seed(&mut db);
    let q = Parser::parse("SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] OR cat = 'b'")
        .expect("test: parse");
    let params = parse_params(Some(
        r#"{"a": [1.0, 0.0, 0.0, 0.0], "b": [0.0, 1.0, 0.0, 0.0]}"#,
    ))
    .expect("test: params");
    let fused = find_fused_search(q.select.where_clause.as_ref()).expect("test: fused");
    let err = execute_fused_search(&db, &q.select, fused, &params);
    assert!(err.is_err());
    assert!(err.expect_err("test: err").contains("NEAR_FUSED"));
}

#[test]
fn test_fused_rejects_mixed_with_near() {
    let cond =
        parse_where("SELECT * FROM vecs WHERE vector NEAR_FUSED [$a, $b] AND vector NEAR $c");
    let err = validate_fused_structure(cond.as_ref());
    assert!(err.is_err());
}

#[test]
fn test_config_to_strategy_maps_like_core() {
    let mk = |s: &str| FusionConfig {
        strategy: s.to_string(),
        params: std::collections::HashMap::new(),
    };
    assert!(matches!(
        config_to_strategy(&mk("average")),
        FusionStrategy::Average
    ));
    assert!(matches!(
        config_to_strategy(&mk("maximum")),
        FusionStrategy::Maximum
    ));
    assert!(matches!(
        config_to_strategy(&mk("weighted")),
        FusionStrategy::RRF { .. }
    ));
    assert!(matches!(
        config_to_strategy(&mk("rrf")),
        FusionStrategy::RRF { .. }
    ));
}
