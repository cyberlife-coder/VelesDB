//! Tests for `conversion` module - VelesQL to Filter conversion.

use super::{Condition, Value};
use crate::velesql::{
    BetweenCondition, CompareOp, Comparison, InCondition, IsNullCondition, LikeCondition,
    MatchCondition, Value as VelesValue,
};

#[test]
fn test_comparison_eq_integer() {
    let cmp = Comparison {
        column: "age".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Integer(25),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::Eq { field, value } if field == "age" && value == Value::Number(25.into()))
    );
}

#[test]
fn test_comparison_neq_string() {
    let cmp = Comparison {
        column: "status".to_string(),
        operator: CompareOp::NotEq,
        value: VelesValue::String("inactive".to_string()),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::Neq { field, value } if field == "status" && value == Value::String("inactive".to_string()))
    );
}

#[test]
fn test_comparison_gt_float() {
    let cmp = Comparison {
        column: "price".to_string(),
        operator: CompareOp::Gt,
        value: VelesValue::Float(99.99),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Gt { field, .. } if field == "price"));
}

#[test]
fn test_comparison_gte() {
    let cmp = Comparison {
        column: "count".to_string(),
        operator: CompareOp::Gte,
        value: VelesValue::Integer(10),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Gte { field, .. } if field == "count"));
}

#[test]
fn test_comparison_lt() {
    let cmp = Comparison {
        column: "score".to_string(),
        operator: CompareOp::Lt,
        value: VelesValue::Integer(50),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Lt { field, .. } if field == "score"));
}

#[test]
fn test_comparison_lte() {
    let cmp = Comparison {
        column: "level".to_string(),
        operator: CompareOp::Lte,
        value: VelesValue::Integer(5),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Lte { field, .. } if field == "level"));
}

#[test]
fn test_comparison_boolean() {
    let cmp = Comparison {
        column: "active".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Boolean(true),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::Eq { field, value } if field == "active" && value == Value::Bool(true))
    );
}

#[test]
fn test_comparison_null() {
    let cmp = Comparison {
        column: "field".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Null,
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Eq { value, .. } if value == Value::Null));
}

#[test]
fn test_in_condition() {
    let inc = InCondition {
        column: "category".to_string(),
        values: vec![
            VelesValue::String("a".to_string()),
            VelesValue::String("b".to_string()),
        ],
        negated: false,
    };
    let cond = crate::velesql::Condition::In(inc);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::In { field, values } if field == "category" && values.len() == 2)
    );
}

#[test]
fn test_not_in_condition() {
    let inc = InCondition {
        column: "status".to_string(),
        values: vec![
            VelesValue::String("draft".to_string()),
            VelesValue::String("deleted".to_string()),
        ],
        negated: true,
    };
    let cond = crate::velesql::Condition::In(inc);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::Not { ref condition } if matches!(**condition, Condition::In { ref field, .. } if field == "status")),
        "NOT IN should convert to Not(In(...))"
    );
}

#[test]
fn test_is_null_true() {
    let isn = IsNullCondition {
        column: "optional".to_string(),
        is_null: true,
    };
    let cond = crate::velesql::Condition::IsNull(isn);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::IsNull { field } if field == "optional"));
}

#[test]
fn test_is_null_false() {
    let isn = IsNullCondition {
        column: "required".to_string(),
        is_null: false,
    };
    let cond = crate::velesql::Condition::IsNull(isn);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::IsNotNull { field } if field == "required"));
}

#[test]
fn test_and_condition() {
    let left = crate::velesql::Condition::Comparison(Comparison {
        column: "a".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Integer(1),
    });
    let right = crate::velesql::Condition::Comparison(Comparison {
        column: "b".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Integer(2),
    });
    let cond = crate::velesql::Condition::And(Box::new(left), Box::new(right));
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
}

#[test]
fn test_or_condition() {
    let left = crate::velesql::Condition::Comparison(Comparison {
        column: "x".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Integer(1),
    });
    let right = crate::velesql::Condition::Comparison(Comparison {
        column: "y".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Integer(2),
    });
    let cond = crate::velesql::Condition::Or(Box::new(left), Box::new(right));
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Or { conditions } if conditions.len() == 2));
}

#[test]
fn test_not_condition() {
    let inner = crate::velesql::Condition::Comparison(Comparison {
        column: "deleted".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::Boolean(true),
    });
    let cond = crate::velesql::Condition::Not(Box::new(inner));
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Not { .. }));
}

#[test]
fn test_group_condition() {
    let inner = crate::velesql::Condition::Comparison(Comparison {
        column: "val".to_string(),
        operator: CompareOp::Gt,
        value: VelesValue::Integer(0),
    });
    let cond = crate::velesql::Condition::Group(Box::new(inner));
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::Gt { field, .. } if field == "val"));
}

#[test]
fn test_match_condition() {
    let m = MatchCondition {
        column: "text".to_string(),
        query: "hello".to_string(),
    };
    let cond = crate::velesql::Condition::Match(m);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::Contains { field, value } if field == "text" && value == "hello")
    );
}

#[test]
fn test_between_condition_integers() {
    let btw = BetweenCondition {
        column: "age".to_string(),
        low: VelesValue::Integer(18),
        high: VelesValue::Integer(65),
    };
    let cond = crate::velesql::Condition::Between(btw);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
}

#[test]
fn test_between_condition_floats() {
    let btw = BetweenCondition {
        column: "price".to_string(),
        low: VelesValue::Float(10.0),
        high: VelesValue::Float(100.0),
    };
    let cond = crate::velesql::Condition::Between(btw);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
}

#[test]
fn test_like_case_sensitive() {
    let lk = LikeCondition {
        column: "name".to_string(),
        pattern: "%test%".to_string(),
        case_insensitive: false,
    };
    let cond = crate::velesql::Condition::Like(lk);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::Like { field, pattern } if field == "name" && pattern == "%test%")
    );
}

#[test]
fn test_like_case_insensitive() {
    let lk = LikeCondition {
        column: "title".to_string(),
        pattern: "%search%".to_string(),
        case_insensitive: true,
    };
    let cond = crate::velesql::Condition::Like(lk);
    let result: Condition = cond.into();
    assert!(
        matches!(result, Condition::ILike { field, pattern } if field == "title" && pattern == "%search%")
    );
}

// ============================================================================
// Issue #512: IN list pre-sorting and deduplication
// ============================================================================

/// GIVEN an IN condition with unordered string values
/// WHEN converted to Condition::In
/// THEN values are sorted lexicographically (enables binary search in matching.rs)
#[test]
fn test_in_values_are_sorted_at_conversion() {
    let inc = InCondition {
        column: "tag".to_string(),
        values: vec![
            VelesValue::String("zoo".to_string()),
            VelesValue::String("apple".to_string()),
            VelesValue::String("mango".to_string()),
        ],
        negated: false,
    };
    let result: Condition = crate::velesql::Condition::In(inc).into();
    if let Condition::In { values, .. } = result {
        let strings: Vec<&str> = values.iter().filter_map(|v| v.as_str()).collect();
        assert_eq!(strings, vec!["apple", "mango", "zoo"], "must be sorted");
    } else {
        panic!("expected Condition::In");
    }
}

/// GIVEN an IN condition with duplicate string values
/// WHEN converted to Condition::In
/// THEN exact duplicates are removed
#[test]
fn test_in_values_are_deduplicated_at_conversion() {
    let inc = InCondition {
        column: "cat".to_string(),
        values: vec![
            VelesValue::String("a".to_string()),
            VelesValue::String("b".to_string()),
            VelesValue::String("a".to_string()),
        ],
        negated: false,
    };
    let result: Condition = crate::velesql::Condition::In(inc).into();
    if let Condition::In { values, .. } = result {
        assert_eq!(values.len(), 2, "duplicate 'a' must be removed");
    } else {
        panic!("expected Condition::In");
    }
}

/// GIVEN an IN condition with integer values out of order
/// WHEN converted to Condition::In
/// THEN values are sorted numerically
#[test]
fn test_in_integer_values_sorted_numerically() {
    let inc = InCondition {
        column: "id".to_string(),
        values: vec![
            VelesValue::Integer(30),
            VelesValue::Integer(10),
            VelesValue::Integer(20),
        ],
        negated: false,
    };
    let result: Condition = crate::velesql::Condition::In(inc).into();
    if let Condition::In { values, .. } = result {
        let nums: Vec<i64> = values.iter().filter_map(|v| v.as_i64()).collect();
        assert_eq!(nums, vec![10, 20, 30], "must be sorted numerically");
    } else {
        panic!("expected Condition::In");
    }
}

/// GIVEN an IN condition with > 16 values (binary search threshold)
/// WHEN matching against a payload containing one of the values
/// THEN the match succeeds (binary search correctness)
#[test]
fn test_in_large_list_binary_search_finds_value() {
    use serde_json::json;

    // Build IN list with 20 string values (> IN_BINARY_SEARCH_THRESHOLD=16)
    let values: Vec<VelesValue> = (0..20)
        .map(|i| VelesValue::String(format!("val_{i:02}")))
        .collect();
    let inc = InCondition {
        column: "code".to_string(),
        values,
        negated: false,
    };
    let cond: Condition = crate::velesql::Condition::In(inc).into();
    let payload = json!({"code": "val_07"});
    assert!(
        cond.matches(&payload),
        "val_07 must be found via binary search"
    );
}

/// GIVEN an IN condition with > 16 values
/// WHEN matching against a payload with a value NOT in the list
/// THEN the match fails (binary search correctness for non-members)
#[test]
fn test_in_large_list_binary_search_rejects_absent_value() {
    use serde_json::json;

    let values: Vec<VelesValue> = (0..20)
        .map(|i| VelesValue::String(format!("val_{i:02}")))
        .collect();
    let inc = InCondition {
        column: "code".to_string(),
        values,
        negated: false,
    };
    let cond: Condition = crate::velesql::Condition::In(inc).into();
    let payload = json!({"code": "absent"});
    assert!(!cond.matches(&payload), "absent value must not match");
}

// ============================================================================
// Issue #486: Value::UnsignedInteger filter conversion
// ============================================================================

#[test]
fn test_comparison_eq_unsigned_integer() {
    let cmp = Comparison {
        column: "big_id".to_string(),
        operator: CompareOp::Eq,
        value: VelesValue::UnsignedInteger(9_223_372_036_854_775_808),
    };
    let cond = crate::velesql::Condition::Comparison(cmp);
    let result: Condition = cond.into();
    // UnsignedInteger should convert to a JSON Number with the same u64 value
    if let Condition::Eq { field, value } = &result {
        assert_eq!(field, "big_id");
        assert_eq!(value.as_u64(), Some(9_223_372_036_854_775_808));
    } else {
        panic!("expected Condition::Eq, got {result:?}");
    }
}

#[test]
fn test_unsigned_integer_numeric_to_json() {
    // UnsignedInteger in BETWEEN should produce valid JSON numbers
    let btw = BetweenCondition {
        column: "id".to_string(),
        low: VelesValue::UnsignedInteger(10_000_000_000_000_000_000),
        high: VelesValue::UnsignedInteger(u64::MAX),
    };
    let cond = crate::velesql::Condition::Between(btw);
    let result: Condition = cond.into();
    assert!(matches!(result, Condition::And { conditions } if conditions.len() == 2));
}
