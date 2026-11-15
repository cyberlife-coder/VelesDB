use super::*;

use serde_json::json;

#[test]
fn the_union_keeps_the_nodes_own_fields() {
    let own = json!({"_labels": ["W"], "name": "Paris"});
    let extra = json!({"name": "SHADOWED", "price": 9.5});
    assert_eq!(
        union(Some(&own), Some(&extra)),
        Some(json!({"_labels": ["W"], "name": "Paris", "price": 9.5}))
    );
}

#[test]
fn either_side_alone_is_the_union_and_scalars_never_merge() {
    assert_eq!(union(None, Some(&json!({"b": 2}))), Some(json!({"b": 2})));
    assert_eq!(union(Some(&json!({"a": 1})), None), Some(json!({"a": 1})));
    assert_eq!(
        union(Some(&json!(7)), Some(&json!({"b": 2}))),
        Some(json!(7))
    );
    assert_eq!(
        union(Some(&json!({"a": 1})), Some(&json!([1]))),
        Some(json!({"a": 1}))
    );
}
