#![cfg(all(test, feature = "persistence"))]

use super::*;

#[test]
fn test_sort_aggregation_results_order_by_count_desc_sorts_rows() {
    // ARRANGE
    let mut rows = vec![
        serde_json::json!({"category": "science", "count": 2}),
        serde_json::json!({"category": "tech", "count": 5}),
        serde_json::json!({"category": "history", "count": 3}),
    ];
    let sort_columns = vec![("count".to_string(), true)];

    // ACT
    Collection::sort_aggregation_results(&mut rows, &sort_columns);

    // ASSERT
    let ordered_categories: Vec<&str> = rows
        .iter()
        .map(|row| {
            row.get("category")
                .and_then(serde_json::Value::as_str)
                .expect("category should be a string")
        })
        .collect();
    assert_eq!(ordered_categories, vec!["tech", "history", "science"]);
}
