use super::*;

#[test]
fn test_streaming_config_defaults() {
    let config = StreamingConfig::new(3, 10000, None);
    assert_eq!(config.max_depth, 3);
    assert_eq!(config.max_visited, 10000);
    assert!(config.relationship_types.is_none());
}
