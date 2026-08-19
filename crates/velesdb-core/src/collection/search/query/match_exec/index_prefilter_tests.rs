use super::*;
use crate::velesql::{Comparison, Condition, NodePattern, Value};

#[test]
fn test_split_alias_property_valid() {
    assert_eq!(split_alias_property("n.age"), Some(("n", "age")));
    assert_eq!(
        split_alias_property("doc.metadata.category"),
        Some(("doc", "metadata.category"))
    );
}

#[test]
fn test_split_alias_property_no_dot() {
    assert_eq!(split_alias_property("age"), None);
}

#[test]
fn test_split_alias_property_empty_parts() {
    assert_eq!(split_alias_property(".age"), None);
    assert_eq!(split_alias_property("n."), None);
}

#[test]
fn test_extract_predicates_single_eq() {
    let cond = Condition::Comparison(Comparison {
        column: "n.name".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("Alice".to_string()),
    });
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert_eq!(preds.len(), 1);
    assert_eq!(preds[0].alias, "n");
    assert_eq!(preds[0].property, "name");
    assert!(matches!(preds[0].kind, PredicateKind::Exact(_)));
}

#[test]
fn test_extract_predicates_and_chain() {
    let left = Condition::Comparison(Comparison {
        column: "n.age".to_string(),
        operator: CompareOp::Gt,
        value: Value::Integer(30),
    });
    let right = Condition::Comparison(Comparison {
        column: "n.name".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("Bob".to_string()),
    });
    let cond = Condition::And(Box::new(left), Box::new(right));
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert_eq!(preds.len(), 2);
}

#[test]
fn test_extract_predicates_or_skipped() {
    let left = Condition::Comparison(Comparison {
        column: "n.age".to_string(),
        operator: CompareOp::Gt,
        value: Value::Integer(30),
    });
    let right = Condition::Comparison(Comparison {
        column: "n.name".to_string(),
        operator: CompareOp::Eq,
        value: Value::String("Bob".to_string()),
    });
    let cond = Condition::Or(Box::new(left), Box::new(right));
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert!(
        preds.is_empty(),
        "OR branches cannot be pre-filtered with intersection"
    );
}

#[test]
fn test_extract_predicates_not_eq_skipped() {
    let cond = Condition::Comparison(Comparison {
        column: "n.name".to_string(),
        operator: CompareOp::NotEq,
        value: Value::String("Alice".to_string()),
    });
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert!(preds.is_empty(), "NotEq cannot be pre-filtered");
}

#[test]
fn test_extract_predicates_bare_column_skipped() {
    let cond = Condition::Comparison(Comparison {
        column: "age".to_string(),
        operator: CompareOp::Eq,
        value: Value::Integer(30),
    });
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert!(
        preds.is_empty(),
        "Bare column names without alias prefix are skipped"
    );
}

#[test]
fn test_build_alias_label_map() {
    let pattern = GraphPattern {
        name: None,
        nodes: vec![
            NodePattern::new().with_alias("n").with_label("Person"),
            NodePattern::new().with_alias("m").with_label("Company"),
        ],
        relationships: Vec::new(),
    };
    let map = build_alias_label_map(&pattern);
    assert_eq!(
        map.get("n").map(Vec::as_slice),
        Some(["Person".to_string()].as_slice())
    );
    assert_eq!(
        map.get("m").map(Vec::as_slice),
        Some(["Company".to_string()].as_slice())
    );
}

#[test]
fn test_passes_prefilter_none() {
    assert!(passes_prefilter(None, 42), "None = no filter, allow all");
}

#[test]
fn test_passes_prefilter_some_contains() {
    let set: HashSet<u64> = [1, 2, 3].into_iter().collect();
    assert!(passes_prefilter(Some(&set), 2));
    assert!(!passes_prefilter(Some(&set), 99));
}

#[test]
fn test_intersect_sets_none_acc() {
    let new: HashSet<u64> = [1, 2, 3].into_iter().collect();
    let result = intersect_sets(None, new.clone());
    assert_eq!(result, new);
}

#[test]
fn test_intersect_sets_some_acc() {
    let acc: HashSet<u64> = [1, 2, 3].into_iter().collect();
    let new: HashSet<u64> = [2, 3, 4].into_iter().collect();
    let result = intersect_sets(Some(acc), new);
    let expected: HashSet<u64> = [2, 3].into_iter().collect();
    assert_eq!(result, expected);
}

#[test]
fn test_between_to_predicate() {
    let btw = crate::velesql::BetweenCondition {
        column: "n.age".to_string(),
        low: Value::Integer(20),
        high: Value::Integer(40),
    };
    let params = HashMap::new();
    let pred = between_to_predicate(&btw, &params);
    assert!(pred.is_some());
    let pred = pred.expect("test: should have predicate");
    assert_eq!(pred.alias, "n");
    assert_eq!(pred.property, "age");
    assert!(matches!(pred.kind, PredicateKind::Range(_, _)));
}

// Regression tests for Devin finding: GTE/LTE must use inclusive bounds,
// not strict GT/LT (which would exclude boundary values from prefilter).

#[test]
fn test_gte_maps_to_gte_not_gt() {
    let cond = Condition::Comparison(Comparison {
        column: "n.age".to_string(),
        operator: CompareOp::Gte,
        value: Value::Integer(30),
    });
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert_eq!(preds.len(), 1);
    assert!(
        matches!(preds[0].kind, PredicateKind::Gte(_)),
        "GTE must map to PredicateKind::Gte (inclusive), not Gt (exclusive)"
    );
}

#[test]
fn test_lte_maps_to_lte_not_lt() {
    let cond = Condition::Comparison(Comparison {
        column: "n.price".to_string(),
        operator: CompareOp::Lte,
        value: Value::Float(99.99),
    });
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert_eq!(preds.len(), 1);
    assert!(
        matches!(preds[0].kind, PredicateKind::Lte(_)),
        "LTE must map to PredicateKind::Lte (inclusive), not Lt (exclusive)"
    );
}

#[test]
fn test_strict_gt_maps_to_gt() {
    let cond = Condition::Comparison(Comparison {
        column: "n.score".to_string(),
        operator: CompareOp::Gt,
        value: Value::Float(0.5),
    });
    let params = HashMap::new();
    let preds = extract_predicates(&cond, &params);
    assert_eq!(preds.len(), 1);
    assert!(
        matches!(preds[0].kind, PredicateKind::Gt(_)),
        "Strict GT must map to PredicateKind::Gt (exclusive)"
    );
}
