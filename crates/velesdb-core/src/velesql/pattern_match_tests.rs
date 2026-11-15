use super::*;

use serde_json::json;

fn props(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

fn holds(properties: &HashMap<String, Value>, id: u64, payload: &serde_json::Value) -> bool {
    inline_properties_hold(properties, |key| {
        element_property(id, key, |k| payload.get(k))
    })
}

#[test]
fn every_pair_must_be_equal() {
    let payload = json!({"name": "Ann", "age": 30});
    let both = props(&[
        ("name", Value::String("Ann".into())),
        ("age", Value::Integer(30)),
    ]);
    assert!(holds(&both, 1, &payload));
    let one_off = props(&[
        ("name", Value::String("Ann".into())),
        ("age", Value::Integer(31)),
    ]);
    assert!(!holds(&one_off, 1, &payload));
    assert!(
        holds(&HashMap::new(), 1, &payload),
        "no property: no constraint"
    );
}

/// Numbers compare by exact value, as WHERE `=` does.
#[test]
fn numbers_compare_by_exact_value() {
    let payload = json!({"x": 1.0, "big": 9_007_199_254_740_993_u64});
    assert!(holds(&props(&[("x", Value::Integer(1))]), 1, &payload));
    assert!(!holds(
        &props(&[("big", Value::Float(9_007_199_254_740_992.0))]),
        1,
        &payload
    ));
}

/// Three-valued, as `n.k = v`: a null or absent property, or a NULL literal,
/// is UNKNOWN and does not match.
#[test]
fn null_and_absent_never_match() {
    let payload = json!({"x": null});
    assert!(!holds(&props(&[("x", Value::Null)]), 1, &payload));
    assert!(!holds(&props(&[("x", Value::Integer(1))]), 1, &payload));
    assert!(!holds(
        &props(&[("missing", Value::Integer(1))]),
        1,
        &payload
    ));
}

/// `id` is the element's id, as `WHERE n.id` reads it.
#[test]
fn id_reads_the_element_id() {
    let payload = json!({"id": 99});
    assert!(holds(&props(&[("id", Value::Integer(7))]), 7, &payload));
    assert!(!holds(&props(&[("id", Value::Integer(99))]), 7, &payload));
}

#[test]
fn every_label_is_required() {
    let required = vec!["A".to_string(), "B".to_string()];
    assert!(has_every_label(&required, &["B", "A", "C"]));
    assert!(!has_every_label(&required, &["A"]));
    assert!(has_every_label(&[], &["A"]));
}
