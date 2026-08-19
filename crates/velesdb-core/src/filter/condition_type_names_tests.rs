use super::Condition;
use crate::CONDITION_TYPE_NAMES;

/// Maps every variant to its serde `type` tag via an exhaustive `match`
/// (no wildcard arm), so adding a variant fails to compile until it is
/// listed here, which in turn flags the missing `CONDITION_TYPE_NAMES`
/// entry asserted below.
fn expected_tag(condition: &Condition) -> &'static str {
    match condition {
        Condition::Eq { .. } => "eq",
        Condition::Neq { .. } => "neq",
        Condition::Gt { .. } => "gt",
        Condition::Gte { .. } => "gte",
        Condition::Lt { .. } => "lt",
        Condition::Lte { .. } => "lte",
        Condition::In { .. } => "in",
        Condition::Contains { .. } => "contains",
        Condition::IsNull { .. } => "is_null",
        Condition::IsNotNull { .. } => "is_not_null",
        Condition::And { .. } => "and",
        Condition::Or { .. } => "or",
        Condition::Not { .. } => "not",
        Condition::Like { .. } => "like",
        Condition::ILike { .. } => "ilike",
        Condition::ArrayContains { .. } => "array_contains",
        Condition::ArrayContainsAny { .. } => "array_contains_any",
        Condition::ArrayContainsAll { .. } => "array_contains_all",
        Condition::GeoDistance { .. } => "geo_distance",
        Condition::GeoBbox { .. } => "geo_bbox",
    }
}

/// Comparison / null / membership variants, in declaration order.
fn comparison_variants() -> Vec<Condition> {
    use serde_json::Value;
    let f = || "f".to_string();
    let cmp = |c: fn(String, Value) -> Condition| c(f(), Value::Null);
    vec![
        cmp(|field, value| Condition::Eq { field, value }),
        cmp(|field, value| Condition::Neq { field, value }),
        cmp(|field, value| Condition::Gt { field, value }),
        cmp(|field, value| Condition::Gte { field, value }),
        cmp(|field, value| Condition::Lt { field, value }),
        cmp(|field, value| Condition::Lte { field, value }),
        Condition::In {
            field: f(),
            values: vec![],
        },
        Condition::Contains {
            field: f(),
            value: String::new(),
        },
        Condition::IsNull { field: f() },
        Condition::IsNotNull { field: f() },
    ]
}

/// Logical, pattern, array and geo variants, in declaration order.
fn logical_and_geo_variants() -> Vec<Condition> {
    use serde_json::Value;
    let f = || "f".to_string();
    vec![
        Condition::And { conditions: vec![] },
        Condition::Or { conditions: vec![] },
        Condition::Not {
            condition: Box::new(Condition::IsNull { field: f() }),
        },
        Condition::Like {
            field: f(),
            pattern: String::new(),
        },
        Condition::ILike {
            field: f(),
            pattern: String::new(),
        },
        Condition::ArrayContains {
            field: f(),
            value: Value::Null,
        },
        Condition::ArrayContainsAny {
            field: f(),
            values: vec![],
        },
        Condition::ArrayContainsAll {
            field: f(),
            values: vec![],
        },
        Condition::GeoDistance {
            field: f(),
            lat: 0.0,
            lng: 0.0,
            operator: crate::velesql::CompareOp::Lt,
            threshold: 0.0,
        },
        Condition::GeoBbox {
            field: f(),
            lat_min: 0.0,
            lng_min: 0.0,
            lat_max: 0.0,
            lng_max: 0.0,
        },
    ]
}

#[test]
fn condition_type_names_match_serde_tags_in_order() {
    let mut variants = comparison_variants();
    variants.extend(logical_and_geo_variants());
    assert_eq!(variants.len(), CONDITION_TYPE_NAMES.len());
    for (i, variant) in variants.iter().enumerate() {
        let serialized = serde_json::to_value(variant).expect("serialize condition");
        let serde_tag = serialized["type"].as_str().expect("type tag present");
        assert_eq!(serde_tag, expected_tag(variant));
        assert_eq!(CONDITION_TYPE_NAMES[i], serde_tag);
    }
}
