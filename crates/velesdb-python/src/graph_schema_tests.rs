use super::*;

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
