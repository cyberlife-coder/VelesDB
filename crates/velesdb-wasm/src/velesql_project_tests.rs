use super::*;

fn rows() -> Vec<OwnedScanRow> {
    vec![
        (1, 0.9, Some(serde_json::json!({"cat": "a", "title": "X"}))),
        (2, 0.5, Some(serde_json::json!({"cat": "b", "title": "Y"}))),
    ]
}

#[test]
fn test_to_search_results_preserves_id_score_payload() {
    let results = to_search_results(rows());
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].point.id, 1);
    assert!((results[0].score - 0.9).abs() < f32::EPSILON);
    assert_eq!(results[1].point.payload.as_ref().unwrap()["cat"], "b");
}

#[test]
fn test_project_specific_columns_only() {
    let mut stmt = SelectStatement::empty();
    stmt.columns = SelectColumns::Columns(vec![velesdb_core::velesql::Column::new("cat")]);
    let results = to_search_results(rows());
    let out = project(&stmt, &results).expect("test: project");
    let body: serde_json::Value = serde_json::from_str(out[0].data_json_ref()).expect("test: json");
    let obj = body.as_object().expect("test: obj");
    assert_eq!(obj.keys().collect::<Vec<_>>(), vec!["cat"]);
    // Real id/score are preserved on the row handle even though the body
    // only carries the projected column.
    assert_eq!(out[0].id(), 1);
}

#[test]
fn test_project_alias_renames() {
    let mut stmt = SelectStatement::empty();
    stmt.columns = SelectColumns::Columns(vec![velesdb_core::velesql::Column::with_alias(
        "title", "name",
    )]);
    let results = to_search_results(rows());
    let out = project(&stmt, &results).expect("test: project");
    let body: serde_json::Value = serde_json::from_str(out[0].data_json_ref()).expect("test: json");
    assert!(body.get("name").is_some());
    assert!(body.get("title").is_none());
}

#[test]
fn test_inject_window_functions_noop_without_window() {
    let stmt = SelectStatement::empty();
    let mut results = to_search_results(rows());
    inject_window_functions(&stmt, &mut results).expect("test: noop");
    // Payload untouched.
    assert_eq!(results[0].point.payload.as_ref().unwrap()["cat"], "a");
}
