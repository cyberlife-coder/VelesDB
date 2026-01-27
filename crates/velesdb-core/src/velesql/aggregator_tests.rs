//! Tests for `aggregator` module - Aggregation functions.

use super::aggregator::*;

#[test]
fn test_aggregator_count() {
    let mut agg = Aggregator::new();
    agg.process_count();
    agg.process_count();
    agg.process_count();

    let result = agg.finalize();
    assert_eq!(result.count, 3);
}

#[test]
fn test_aggregator_sum() {
    let mut agg = Aggregator::new();
    agg.process_value("price", &serde_json::json!(10));
    agg.process_value("price", &serde_json::json!(20));
    agg.process_value("price", &serde_json::json!(30));

    let result = agg.finalize();
    assert_eq!(result.sums.get("price"), Some(&60.0));
}

#[test]
fn test_aggregator_avg() {
    let mut agg = Aggregator::new();
    agg.process_value("rating", &serde_json::json!(3));
    agg.process_value("rating", &serde_json::json!(4));
    agg.process_value("rating", &serde_json::json!(5));

    let result = agg.finalize();
    assert_eq!(result.avgs.get("rating"), Some(&4.0));
}

#[test]
fn test_aggregator_min_max() {
    let mut agg = Aggregator::new();
    agg.process_value("val", &serde_json::json!(5));
    agg.process_value("val", &serde_json::json!(1));
    agg.process_value("val", &serde_json::json!(9));

    let result = agg.finalize();
    assert_eq!(result.mins.get("val"), Some(&1.0));
    assert_eq!(result.maxs.get("val"), Some(&9.0));
}

#[test]
fn test_aggregator_multiple_columns() {
    let mut agg = Aggregator::new();
    agg.process_count();
    agg.process_value("a", &serde_json::json!(10));
    agg.process_value("b", &serde_json::json!(100));
    agg.process_count();
    agg.process_value("a", &serde_json::json!(20));
    agg.process_value("b", &serde_json::json!(200));

    let result = agg.finalize();
    assert_eq!(result.count, 2);
    assert_eq!(result.sums.get("a"), Some(&30.0));
    assert_eq!(result.sums.get("b"), Some(&300.0));
}

#[test]
fn test_result_to_json() {
    let mut agg = Aggregator::new();
    agg.process_count();
    agg.process_value("price", &serde_json::json!(50));

    let result = agg.finalize();
    let json = result.to_json();

    assert_eq!(
        json.get("count").and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        json.get("sum_price").and_then(serde_json::Value::as_f64),
        Some(50.0)
    );
}

#[test]
fn test_process_batch_sum() {
    let mut agg = Aggregator::new();
    let values: Vec<f64> = (1..=100).map(|x| x as f64).collect();
    agg.process_batch("value", &values);

    let result = agg.finalize();
    assert_eq!(result.sums.get("value"), Some(&5050.0));
    assert_eq!(result.counts.get("value"), Some(&100));
}

#[test]
fn test_process_batch_min_max() {
    let mut agg = Aggregator::new();
    let values = vec![5.0, 1.0, 9.0, 3.0, 7.0];
    agg.process_batch("val", &values);

    let result = agg.finalize();
    assert_eq!(result.mins.get("val"), Some(&1.0));
    assert_eq!(result.maxs.get("val"), Some(&9.0));
}

#[test]
fn test_process_batch_multiple_batches() {
    let mut agg = Aggregator::new();
    agg.process_batch("x", &[1.0, 2.0, 3.0]);
    agg.process_batch("x", &[4.0, 5.0, 6.0]);

    let result = agg.finalize();
    assert_eq!(result.sums.get("x"), Some(&21.0));
    assert_eq!(result.counts.get("x"), Some(&6));
    assert_eq!(result.mins.get("x"), Some(&1.0));
    assert_eq!(result.maxs.get("x"), Some(&6.0));
}

#[test]
fn test_process_batch_equivalence_with_process_value() {
    let values = vec![10.0, 20.0, 30.0, 40.0, 50.0];

    let mut batch_agg = Aggregator::new();
    batch_agg.process_batch("price", &values);

    let mut value_agg = Aggregator::new();
    for v in &values {
        value_agg.process_value("price", &serde_json::json!(v));
    }

    let batch_result = batch_agg.finalize();
    let value_result = value_agg.finalize();

    assert_eq!(
        batch_result.sums.get("price"),
        value_result.sums.get("price")
    );
    assert_eq!(
        batch_result.counts.get("price"),
        value_result.counts.get("price")
    );
    assert_eq!(
        batch_result.mins.get("price"),
        value_result.mins.get("price")
    );
    assert_eq!(
        batch_result.maxs.get("price"),
        value_result.maxs.get("price")
    );
}
