//! Tests for `match_exec` module - MATCH clause execution.

use super::match_exec::*;
use std::collections::HashMap;

#[test]
fn test_match_result_creation() {
    let result = MatchResult::new(42, 2, vec![1, 2]);
    assert_eq!(result.node_id, 42);
    assert_eq!(result.depth, 2);
    assert_eq!(result.path, vec![1, 2]);
    // new() must leave all optional collections empty / score unset
    assert!(result.bindings.is_empty());
    assert!(result.edge_bindings.is_empty());
    assert!(result.edge_paths.is_empty());
    assert!(result.projected.is_empty());
    assert!(result.score.is_none());
}

#[test]
fn test_match_result_with_binding() {
    // Accumulates distinct aliases.
    let result = MatchResult::new(42, 0, vec![])
        .with_binding("n".to_string(), 42)
        .with_binding("m".to_string(), 7);
    assert_eq!(result.bindings.len(), 2);
    assert_eq!(result.bindings.get("n"), Some(&42));
    assert_eq!(result.bindings.get("m"), Some(&7));
    // Re-binding the same alias overwrites (last write wins), does not duplicate.
    let result = result.with_binding("n".to_string(), 99);
    assert_eq!(result.bindings.len(), 2);
    assert_eq!(result.bindings.get("n"), Some(&99));
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
    // with_projected (chained after with_binding) must not disturb the prior binding.
    assert_eq!(result.bindings.get("doc"), Some(&42));
    // The projected map is a replacement, not a merge: only the inserted keys exist.
    assert!(!result.projected.contains_key("missing.key"));
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

// ============================================================================
// Fix #489: ProjectionItem parsing tests
// ============================================================================

#[test]
fn test_parse_projection_wildcard() {
    let item = parse_projection_item("*");
    assert!(
        matches!(item, ProjectionItem::Wildcard),
        "Expected Wildcard, got {item:?}"
    );
}

#[test]
fn test_parse_projection_similarity_function() {
    let item = parse_projection_item("similarity()");
    assert!(
        matches!(item, ProjectionItem::FunctionCall("similarity")),
        "Expected FunctionCall(\"similarity\"), got {item:?}"
    );
}

#[test]
fn test_parse_projection_count_function() {
    let item = parse_projection_item("count()");
    assert!(
        matches!(item, ProjectionItem::FunctionCall("count")),
        "Expected FunctionCall(\"count\"), got {item:?}"
    );
}

#[test]
fn test_parse_projection_bare_alias() {
    let item = parse_projection_item("n");
    assert!(
        matches!(item, ProjectionItem::BareAlias("n")),
        "Expected BareAlias(\"n\"), got {item:?}"
    );
}

#[test]
fn test_parse_projection_bare_alias_longer_name() {
    let item = parse_projection_item("author");
    assert!(
        matches!(item, ProjectionItem::BareAlias("author")),
        "Expected BareAlias(\"author\"), got {item:?}"
    );
}

#[test]
fn test_parse_projection_property_path() {
    let item = parse_projection_item("n.name");
    match item {
        ProjectionItem::PropertyPath { alias, property } => {
            assert_eq!(alias, "n");
            assert_eq!(property, "name");
        }
        other => panic!("Expected PropertyPath, got {other:?}"),
    }
}

#[test]
fn test_parse_projection_nested_path() {
    let item = parse_projection_item("doc.metadata.category");
    match item {
        ProjectionItem::PropertyPath { alias, property } => {
            assert_eq!(alias, "doc");
            assert_eq!(property, "metadata.category");
        }
        other => panic!("Expected PropertyPath, got {other:?}"),
    }
}

#[test]
fn test_parse_projection_edge_leading_dot() {
    // ".name" — invalid, leading dot with no alias
    let item = parse_projection_item(".name");
    assert!(
        matches!(item, ProjectionItem::BareAlias(_)),
        "Leading dot with no valid split should fall through to BareAlias, got {item:?}"
    );
}

#[test]
fn test_parse_projection_edge_trailing_dot() {
    // "alias." — trailing dot with no property
    let item = parse_projection_item("alias.");
    assert!(
        matches!(item, ProjectionItem::BareAlias(_)),
        "Trailing dot with no property should fall through to BareAlias, got {item:?}"
    );
}
