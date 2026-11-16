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
fn test_build_traversal_config_min_depth_and_deadline() {
    let config = build_traversal_config(Some(2), Some(5), Some(50), None, Some(250));
    assert_eq!(config.min_depth, 2);
    assert!(config.deadline.is_some());
    let config = build_traversal_config(None, None, None, None, None);
    assert_eq!(config.min_depth, 1);
    assert!(config.deadline.is_none());
}
