//! Unit tests for the condition → bitmap translation layer.
//!
//! The contract under test: returned bitmaps must never miss a row the JSON
//! filter would match (false negatives forbidden); `exact` must only be true
//! when the bitmap is precisely the JSON match set.

use super::translate::condition_bitmap;
use super::MirrorState;
use crate::filter::Condition;
use serde_json::json;

/// Builds a mirror with rows:
/// 0: {category: "tech",   price: 10,   active: true}
/// 1: {category: "bio",    price: 20.5, active: false}
/// 2: {category: "tech",   price: 30}
/// 3: {price: "not-a-number"}            (type conflict on price)
/// 4: `{tags: ["a"], meta: {"x": 1}}`      (non-scalars only)
/// 5: no payload
fn sample_state() -> MirrorState {
    let mut state = MirrorState::default();
    assert!(state.upsert_row(
        0,
        Some(&json!({"category": "tech", "price": 10, "active": true}))
    ));
    assert!(state.upsert_row(
        1,
        Some(&json!({"category": "bio", "price": 20.5, "active": false}))
    ));
    assert!(state.upsert_row(2, Some(&json!({"category": "tech", "price": 30}))));
    assert!(state.upsert_row(3, Some(&json!({"price": "not-a-number"}))));
    assert!(state.upsert_row(4, Some(&json!({"tags": ["a"], "meta": {"x": 1}}))));
    assert!(state.upsert_row(5, None));
    state
}

fn rows(eval: &super::translate::Eval) -> Vec<u32> {
    eval.bits.iter().collect()
}

#[test]
fn eq_string_matches_exact_rows() {
    let state = sample_state();
    let cond = Condition::Eq {
        field: "category".into(),
        value: json!("tech"),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![0, 2]);
    assert!(eval.exact);
}

#[test]
fn eq_number_uses_epsilon_semantics() {
    let state = sample_state();
    let cond = Condition::Eq {
        field: "price".into(),
        value: json!(20.5),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![1]);
    assert!(eval.exact);
}

#[test]
fn eq_bool_matches() {
    let state = sample_state();
    let cond = Condition::Eq {
        field: "active".into(),
        value: json!(true),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![0]);
    assert!(eval.exact);
}

#[test]
fn eq_absent_field_is_exact_empty() {
    let state = sample_state();
    let cond = Condition::Eq {
        field: "nonexistent".into(),
        value: json!("x"),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert!(eval.bits.is_empty());
    assert!(eval.exact);
}

#[test]
fn eq_on_array_only_field_is_exact_empty() {
    // "tags" exists only as an array → never mirrored as a column; a scalar
    // Eq can never match it in JSON semantics either.
    let state = sample_state();
    let cond = Condition::Eq {
        field: "tags".into(),
        value: json!("a"),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert!(eval.bits.is_empty());
    assert!(eval.exact);
}

#[test]
fn dotted_field_falls_back() {
    let state = sample_state();
    let cond = Condition::Eq {
        field: "meta.x".into(),
        value: json!(1),
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}

#[test]
fn type_mismatched_literal_falls_back() {
    // price column is Float, but row 3 holds a string — a string literal
    // could match that null-mirrored row, so the leaf must fall back.
    let state = sample_state();
    let cond = Condition::Eq {
        field: "price".into(),
        value: json!("not-a-number"),
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}

#[test]
fn ordering_on_numbers_matches_compare_values() {
    let state = sample_state();
    let cond = Condition::Gte {
        field: "price".into(),
        value: json!(20),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    // Row 3 holds a string price (null cell) and must not match.
    assert_eq!(rows(&eval), vec![1, 2]);
    assert!(eval.exact);
}

#[test]
fn ordering_with_bool_literal_is_exact_empty() {
    // JSON compare_values never orders against a Bool → no row matches.
    let state = sample_state();
    let cond = Condition::Gt {
        field: "price".into(),
        value: json!(true),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert!(eval.bits.is_empty());
    assert!(eval.exact);
}

#[test]
fn string_ordering_falls_back() {
    let state = sample_state();
    let cond = Condition::Gt {
        field: "category".into(),
        value: json!("a"),
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}

#[test]
fn neq_complements_over_live_rows() {
    let state = sample_state();
    let cond = Condition::Neq {
        field: "category".into(),
        value: json!("tech"),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    // JSON Neq matches rows where the field differs OR is missing.
    assert_eq!(rows(&eval), vec![1, 3, 4, 5]);
    assert!(eval.exact);
}

#[test]
fn neq_excludes_tombstoned_rows() {
    let mut state = sample_state();
    state.tombstone(5);
    let cond = Condition::Neq {
        field: "category".into(),
        value: json!("tech"),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![1, 3, 4]);
}

#[test]
fn in_list_on_strings_and_numbers() {
    let state = sample_state();
    let cond = Condition::In {
        field: "category".into(),
        values: vec![json!("bio"), json!("nope")],
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![1]);

    let cond = Condition::In {
        field: "price".into(),
        values: vec![json!(10), json!(30)],
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![0, 2]);
    assert!(eval.exact);
}

#[test]
fn in_list_with_mixed_types_falls_back() {
    let state = sample_state();
    let cond = Condition::In {
        field: "price".into(),
        values: vec![json!(10), json!("not-a-number")],
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}

#[test]
fn empty_in_list_is_exact_empty() {
    let state = sample_state();
    let cond = Condition::In {
        field: "category".into(),
        values: vec![],
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert!(eval.bits.is_empty());
    assert!(eval.exact);
}

#[test]
fn and_with_unsupported_branch_is_inexact_superset() {
    let state = sample_state();
    let cond = Condition::And {
        conditions: vec![
            Condition::Eq {
                field: "category".into(),
                value: json!("tech"),
            },
            Condition::Like {
                field: "category".into(),
                pattern: "%ech".into(),
            },
        ],
    };
    let eval = condition_bitmap(&state, &cond).expect("supported branch present");
    assert_eq!(rows(&eval), vec![0, 2]); // superset from the Eq branch
    assert!(!eval.exact);
}

#[test]
fn empty_and_is_identity_over_live_rows() {
    let state = sample_state();
    let cond = Condition::And { conditions: vec![] };
    let eval = condition_bitmap(&state, &cond).expect("identity");
    assert_eq!(rows(&eval), vec![0, 1, 2, 3, 4, 5]);
    assert!(eval.exact);
}

#[test]
fn or_with_unsupported_branch_falls_back() {
    let state = sample_state();
    let cond = Condition::Or {
        conditions: vec![
            Condition::Eq {
                field: "category".into(),
                value: json!("tech"),
            },
            Condition::Like {
                field: "category".into(),
                pattern: "%bio".into(),
            },
        ],
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}

#[test]
fn or_unions_branches() {
    let state = sample_state();
    let cond = Condition::Or {
        conditions: vec![
            Condition::Eq {
                field: "category".into(),
                value: json!("bio"),
            },
            Condition::Gt {
                field: "price".into(),
                value: json!(25),
            },
        ],
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![1, 2]);
    assert!(eval.exact);
}

#[test]
fn not_over_inexact_falls_back() {
    let state = sample_state();
    let cond = Condition::Not {
        condition: Box::new(Condition::And {
            conditions: vec![
                Condition::Eq {
                    field: "category".into(),
                    value: json!("tech"),
                },
                Condition::Like {
                    field: "category".into(),
                    pattern: "%x".into(),
                },
            ],
        }),
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}

#[test]
fn not_over_exact_complements() {
    let state = sample_state();
    let cond = Condition::Not {
        condition: Box::new(Condition::In {
            field: "category".into(),
            values: vec![json!("tech")],
        }),
    };
    let eval = condition_bitmap(&state, &cond).expect("supported");
    assert_eq!(rows(&eval), vec![1, 3, 4, 5]);
    assert!(eval.exact);
}

#[test]
fn upsert_same_id_is_last_writer_wins() {
    let mut state = MirrorState::default();
    assert!(state.upsert_row(7, Some(&json!({"category": "old"}))));
    assert!(state.upsert_row(7, Some(&json!({"category": "new"}))));
    assert_eq!(state.live.len(), 1);

    let old = Condition::Eq {
        field: "category".into(),
        value: json!("old"),
    };
    let eval = condition_bitmap(&state, &old).expect("supported");
    assert!(eval.bits.is_empty());

    let new = Condition::Eq {
        field: "category".into(),
        value: json!("new"),
    };
    let eval = condition_bitmap(&state, &new).expect("supported");
    assert_eq!(eval.bits.len(), 1);
}

#[test]
fn column_cap_routes_overflow_fields_to_fallback() {
    let mut state = MirrorState::default();
    let mut payload = serde_json::Map::new();
    for i in 0..70 {
        payload.insert(format!("field{i:02}"), json!(i));
    }
    assert!(state.upsert_row(1, Some(&serde_json::Value::Object(payload))));
    assert_eq!(state.store.column_names().count(), 64);
    assert_eq!(state.uncolumnized.len(), 6);

    let capped_field = state
        .uncolumnized
        .iter()
        .next()
        .expect("capped field exists")
        .clone();
    let cond = Condition::Eq {
        field: capped_field,
        value: json!(1),
    };
    assert!(condition_bitmap(&state, &cond).is_none());
}
