//! Tests for `match_exec` module - MATCH clause execution.

use super::match_exec::*;
use std::collections::HashMap;

#[test]
fn test_match_result_creation() {
    let result = MatchResult::new(42, 2, vec![1, 2]);
    assert_eq!(result.node_id, 42);
    assert_eq!(result.depth, 2);
    assert_eq!(result.path, vec![1, 2]);
}

#[test]
fn test_match_result_with_binding() {
    let result = MatchResult::new(42, 0, vec![]).with_binding("n".to_string(), 42);
    assert_eq!(result.bindings.get("n"), Some(&42));
}

// ============================================================================
// Property Projection Tests (EPIC-058 US-007)
// ============================================================================

#[test]
fn test_match_result_with_projected_properties() {
    let mut projected = HashMap::new();
    projected.insert("author.name".to_string(), serde_json::json!("John Doe"));
    projected.insert("doc.title".to_string(), serde_json::json!("Research Paper"));

    let result = MatchResult::new(42, 1, vec![1])
        .with_binding("doc".to_string(), 42)
        .with_projected(projected.clone());

    assert_eq!(result.projected.len(), 2);
    assert_eq!(
        result.projected.get("author.name"),
        Some(&serde_json::json!("John Doe"))
    );
    assert_eq!(
        result.projected.get("doc.title"),
        Some(&serde_json::json!("Research Paper"))
    );
}

#[test]
fn test_parse_property_path_valid() {
    // "author.name" -> ("author", "name")
    let (alias, property) = parse_property_path("author.name").unwrap();
    assert_eq!(alias, "author");
    assert_eq!(property, "name");
}

#[test]
fn test_parse_property_path_nested() {
    // "doc.metadata.category" -> ("doc", "metadata.category")
    let (alias, property) = parse_property_path("doc.metadata.category").unwrap();
    assert_eq!(alias, "doc");
    assert_eq!(property, "metadata.category");
}

#[test]
fn test_parse_property_path_invalid_no_dot() {
    // "nodot" -> None (invalid)
    let result = parse_property_path("nodot");
    assert!(result.is_none());
}

#[test]
fn test_parse_property_path_star() {
    // "*" -> None (wildcard, not a property path)
    let result = parse_property_path("*");
    assert!(result.is_none());
}

#[test]
fn test_parse_property_path_function() {
    // "similarity()" -> None (function, not a property path)
    let result = parse_property_path("similarity()");
    assert!(result.is_none());
}
