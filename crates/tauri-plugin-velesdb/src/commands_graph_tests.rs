use super::build_edge;

#[test]
fn test_build_edge_with_object_properties() {
    let edge = build_edge(1, 10, 20, "KNOWS", Some(serde_json::json!({"weight": 0.5})))
        .expect("valid edge");
    assert_eq!(edge.id(), 1);
    assert_eq!(edge.source(), 10);
    assert_eq!(edge.target(), 20);
    assert_eq!(edge.label(), "KNOWS");
}

#[test]
fn test_build_edge_null_and_non_object_properties_default_empty() {
    // Null and non-object property payloads normalize to no properties
    // rather than erroring.
    assert!(build_edge(2, 1, 2, "L", None).is_ok());
    assert!(build_edge(3, 1, 2, "L", Some(serde_json::Value::Null)).is_ok());
    assert!(build_edge(4, 1, 2, "L", Some(serde_json::json!("scalar"))).is_ok());
}
