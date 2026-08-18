use super::*;

#[test]
fn test_build_traversal_config_defaults() {
    let config = build_traversal_config(None, None, None);
    assert_eq!(config.min_depth, 1);
    assert_eq!(config.max_depth, 3);
    assert_eq!(config.limit, 100);
    assert!(config.rel_types.is_empty());
}

#[test]
fn test_build_traversal_config_custom() {
    let config = build_traversal_config(Some(5), Some(50), Some(vec!["KNOWS".to_string()]));
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
