use super::*;

#[test]
fn test_build_traversal_config_defaults() {
    let config = build_traversal_config(None, None, None, None, None);
    assert_eq!(config.min_depth, 1);
    assert_eq!(config.max_depth, 3);
    assert_eq!(config.limit, 100);
    assert!(config.rel_types.is_empty());
}

#[test]
fn test_build_traversal_config_custom() {
    let config = build_traversal_config(
        None,
        Some(5),
        Some(50),
        Some(vec!["KNOWS".to_string()]),
        None,
    );
    assert_eq!(config.max_depth, 5);
    assert_eq!(config.limit, 50);
    assert_eq!(config.rel_types, vec!["KNOWS"]);
}

#[test]
fn test_py_graph_schema_schemaless() {
    let schema = PyGraphSchema::schemaless();
    assert!(schema.inner.is_schemaless());
}

#[test]
fn test_py_graph_schema_strict() {
    let schema = PyGraphSchema::strict();
    assert!(!schema.inner.is_schemaless());
}

#[test]
fn test_py_graph_schema_strict_with_types() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), "string".to_string());
    props.insert("age".to_string(), "integer".to_string());
    let schema = PyGraphSchema::strict()
        .with_node_type("Person", Some(props))
        .unwrap()
        .with_edge_type("KNOWS", "Person", "Person", None)
        .unwrap();
    assert!(!schema.inner.is_schemaless());
    assert!(schema.inner.has_node_type("Person"));
    assert!(schema.inner.has_edge_type("KNOWS"));
    assert_eq!(schema.inner.node_types().len(), 1);
    assert_eq!(schema.inner.edge_types().len(), 1);
    let person = schema.inner.get_node_type("Person").unwrap();
    assert_eq!(person.property_type("age"), Some(&ValueType::Integer));
}

#[test]
fn test_py_graph_schema_rejects_unknown_value_type() {
    let mut props = HashMap::new();
    props.insert("x".to_string(), "bogus".to_string());
    let err = PyGraphSchema::strict()
        .with_node_type("T", Some(props))
        .err()
        .unwrap();
    assert!(err.to_string().contains("unknown value type"));
}

#[test]
fn test_build_traversal_config_min_depth_and_deadline() {
    let config = build_traversal_config(Some(2), Some(5), Some(50), None, Some(250));
    assert_eq!(config.min_depth, 2);
    assert!(config.deadline.is_some());
    let config = build_traversal_config(None, None, None, None, None);
    assert_eq!(config.min_depth, 1);
    assert!(config.deadline.is_none());
}
