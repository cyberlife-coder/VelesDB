use super::*;

use serde_json::json;

fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("test: an object, got {other}"),
    }
}

#[test]
fn a_base_row_reads_flat_and_under_its_alias() {
    let row = scope_base(Some(json!({"amount": 100})), "o", 1);
    assert_eq!(row, json!({"amount": 100, "o": {"amount": 100, "id": 1}}));
}

/// A self-join's two sides keep their own `name` under their aliases; the
/// flat field is the later side's.
#[test]
fn each_side_keeps_its_fields_under_its_alias() {
    let base = scope_base(Some(json!({"name": "Ann", "boss": 2})), "e", 1);
    let row = merge_side(Some(base), "m", &fields(json!({"name": "Bo"})), Some(2));
    assert_eq!(row["e"]["name"], json!("Ann"));
    assert_eq!(row["m"], json!({"name": "Bo", "id": 2}));
    assert_eq!(row["name"], json!("Bo"));
}

#[test]
fn an_unmatched_side_has_a_null_id() {
    let base = scope_base(Some(json!({"x": 1})), "o", 1);
    let row = merge_side(Some(base), "c", &fields(json!({"name": null})), None);
    assert_eq!(row["c"], json!({"name": null, "id": null}));
    assert_eq!(row["name"], json!(null));
}

#[test]
fn a_side_reads_under_its_table_without_an_alias() {
    assert_eq!(side_alias(None, "orders"), "orders");
    assert_eq!(side_alias(Some("o"), "orders"), "o");
}
