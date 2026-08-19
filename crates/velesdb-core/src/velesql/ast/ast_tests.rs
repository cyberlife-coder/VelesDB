use super::*;

#[test]
fn test_with_clause_new() {
    let clause = WithClause::new();
    assert!(clause.options.is_empty());
}

#[test]
fn test_with_clause_with_option() {
    let clause = WithClause::new()
        .with_option("mode", WithValue::String("accurate".to_string()))
        .with_option("ef_search", WithValue::Integer(512));
    assert_eq!(clause.options.len(), 2);
}

#[test]
fn test_with_clause_get() {
    let clause = WithClause::new().with_option("mode", WithValue::String("fast".to_string()));
    assert!(clause.get("mode").is_some());
    assert!(clause.get("MODE").is_some());
    assert!(clause.get("unknown").is_none());
}

#[test]
fn test_with_clause_get_mode() {
    let clause = WithClause::new().with_option("mode", WithValue::String("accurate".to_string()));
    assert_eq!(clause.get_mode(), Some("accurate"));
}

#[test]
fn test_with_value_as_str() {
    let v = WithValue::String("test".to_string());
    assert_eq!(v.as_str(), Some("test"));
}

#[test]
fn test_with_value_as_integer() {
    let v = WithValue::Integer(100);
    assert_eq!(v.as_integer(), Some(100));
}

#[test]
fn test_with_value_as_float() {
    let v = WithValue::Float(1.234);
    assert!((v.as_float().unwrap() - 1.234).abs() < 1e-5);
}

#[test]
fn test_interval_to_seconds() {
    assert_eq!(
        IntervalValue {
            magnitude: 30,
            unit: IntervalUnit::Seconds
        }
        .to_seconds(),
        30
    );
    assert_eq!(
        IntervalValue {
            magnitude: 1,
            unit: IntervalUnit::Days
        }
        .to_seconds(),
        86400
    );
}

#[test]
fn test_temporal_now() {
    let expr = TemporalExpr::Now;
    let epoch = expr.to_epoch_seconds();
    assert!(epoch > 1_577_836_800);
}

#[test]
fn test_value_from_i64() {
    let v: Value = 42i64.into();
    assert_eq!(v, Value::Integer(42));
}

#[test]
fn test_fusion_config_default() {
    let config = FusionConfig::default();
    assert_eq!(config.strategy, "rrf");
}

#[test]
fn test_fusion_config_rrf() {
    let config = FusionConfig::rrf();
    assert_eq!(config.strategy, "rrf");
    assert!((config.params.get("k").unwrap() - 60.0).abs() < 1e-5);
}

#[test]
fn test_fusion_clause_default() {
    let clause = FusionClause::default();
    assert_eq!(clause.strategy, FusionStrategyType::Rrf);
    assert_eq!(clause.k, Some(60));
}

#[test]
fn test_group_by_clause_default() {
    let clause = GroupByClause::default();
    assert!(clause.columns.is_empty());
}

#[test]
fn test_having_clause_default() {
    let clause = HavingClause::default();
    assert!(clause.conditions.is_empty());
}
