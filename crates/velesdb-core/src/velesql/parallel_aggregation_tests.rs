//! Tests for parallel aggregation (EPIC-018 US-001).
//!
//! TDD: These tests are written BEFORE implementation.

use super::*;

#[test]
fn test_aggregator_merge_counts() {
    let mut agg1 = Aggregator::new();
    agg1.process_count();
    agg1.process_count();

    let mut agg2 = Aggregator::new();
    agg2.process_count();
    agg2.process_count();
    agg2.process_count();

    // Merge agg2 into agg1
    agg1.merge(agg2);

    let result = agg1.finalize();
    assert_eq!(result.count, 5, "Merged count should be 2 + 3 = 5");
}

#[test]
fn test_aggregator_merge_sums() {
    let mut agg1 = Aggregator::new();
    agg1.process_value("price", &serde_json::json!(10.0));
    agg1.process_value("price", &serde_json::json!(20.0));

    let mut agg2 = Aggregator::new();
    agg2.process_value("price", &serde_json::json!(30.0));
    agg2.process_value("price", &serde_json::json!(40.0));

    agg1.merge(agg2);

    let result = agg1.finalize();
    assert_eq!(
        result.sums.get("price"),
        Some(&100.0),
        "Merged sum should be 10+20+30+40 = 100"
    );
}

#[test]
fn test_aggregator_merge_min_max() {
    let mut agg1 = Aggregator::new();
    agg1.process_value("score", &serde_json::json!(50.0));
    agg1.process_value("score", &serde_json::json!(30.0));

    let mut agg2 = Aggregator::new();
    agg2.process_value("score", &serde_json::json!(10.0));
    agg2.process_value("score", &serde_json::json!(80.0));

    agg1.merge(agg2);

    let result = agg1.finalize();
    assert_eq!(
        result.mins.get("score"),
        Some(&10.0),
        "Merged min should be 10"
    );
    assert_eq!(
        result.maxs.get("score"),
        Some(&80.0),
        "Merged max should be 80"
    );
}

#[test]
fn test_aggregator_merge_avg() {
    // agg1: 10, 20 (sum=30, count=2)
    // agg2: 30 (sum=30, count=1)
    // merged: sum=60, count=3, avg=20
    let mut agg1 = Aggregator::new();
    agg1.process_value("value", &serde_json::json!(10.0));
    agg1.process_value("value", &serde_json::json!(20.0));

    let mut agg2 = Aggregator::new();
    agg2.process_value("value", &serde_json::json!(30.0));

    agg1.merge(agg2);

    let result = agg1.finalize();
    assert_eq!(
        result.avgs.get("value"),
        Some(&20.0),
        "Merged avg should be (10+20+30)/3 = 20"
    );
}

#[test]
fn test_aggregator_merge_multiple_columns() {
    let mut agg1 = Aggregator::new();
    agg1.process_value("price", &serde_json::json!(100.0));
    agg1.process_value("qty", &serde_json::json!(5.0));

    let mut agg2 = Aggregator::new();
    agg2.process_value("price", &serde_json::json!(200.0));
    agg2.process_value("qty", &serde_json::json!(10.0));

    agg1.merge(agg2);

    let result = agg1.finalize();
    assert_eq!(result.sums.get("price"), Some(&300.0));
    assert_eq!(result.sums.get("qty"), Some(&15.0));
}

#[test]
fn test_aggregator_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Aggregator>();
}
