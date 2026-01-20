//! Tests for GraphSchema, NodeType, and EdgeType.
//!
//! TDD: Tests written BEFORE implementation (RED phase).

use super::*;
use std::collections::HashMap;

// =============================================================================
// AC-1: Création collection GRAPH avec schéma
// =============================================================================

#[test]
fn test_create_graph_schema_with_node_types() {
    let mut person_props = HashMap::new();
    person_props.insert("name".to_string(), ValueType::String);
    person_props.insert("age".to_string(), ValueType::Integer);

    let mut company_props = HashMap::new();
    company_props.insert("name".to_string(), ValueType::String);
    company_props.insert("founded".to_string(), ValueType::Integer);

    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person").with_properties(person_props))
        .with_node_type(NodeType::new("Company").with_properties(company_props));

    assert_eq!(schema.node_types().len(), 2);
    assert!(schema.has_node_type("Person"));
    assert!(schema.has_node_type("Company"));
    assert!(!schema.has_node_type("Animal"));
}

#[test]
fn test_create_graph_schema_with_edge_types() {
    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person"))
        .with_node_type(NodeType::new("Company"))
        .with_edge_type(EdgeType::new("WORKS_AT", "Person", "Company"))
        .with_edge_type(EdgeType::new("KNOWS", "Person", "Person"));

    assert_eq!(schema.edge_types().len(), 2);
    assert!(schema.has_edge_type("WORKS_AT"));
    assert!(schema.has_edge_type("KNOWS"));
    assert!(!schema.has_edge_type("OWNS"));
}

#[test]
fn test_node_type_with_properties() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), ValueType::String);
    props.insert("score".to_string(), ValueType::Float);
    props.insert("active".to_string(), ValueType::Boolean);

    let node_type = NodeType::new("Entity").with_properties(props);

    assert_eq!(node_type.name(), "Entity");
    assert_eq!(node_type.properties().len(), 3);
    assert_eq!(node_type.property_type("name"), Some(&ValueType::String));
    assert_eq!(node_type.property_type("score"), Some(&ValueType::Float));
    assert_eq!(node_type.property_type("unknown"), None);
}

#[test]
fn test_edge_type_with_properties() {
    let mut props = HashMap::new();
    props.insert("since".to_string(), ValueType::Integer);
    props.insert("role".to_string(), ValueType::String);

    let edge_type = EdgeType::new("WORKS_AT", "Person", "Company").with_properties(props);

    assert_eq!(edge_type.name(), "WORKS_AT");
    assert_eq!(edge_type.from_type(), "Person");
    assert_eq!(edge_type.to_type(), "Company");
    assert_eq!(edge_type.properties().len(), 2);
}

// =============================================================================
// AC-2: Schéma optionnel (schemaless mode)
// =============================================================================

#[test]
fn test_schemaless_graph_schema() {
    let schema = GraphSchema::schemaless();

    assert!(schema.is_schemaless());
    assert!(schema.node_types().is_empty());
    assert!(schema.edge_types().is_empty());
}

#[test]
fn test_schemaless_accepts_any_node_type() {
    let schema = GraphSchema::schemaless();

    // Schemaless mode should accept any node type
    assert!(schema.validate_node_type("Person").is_ok());
    assert!(schema.validate_node_type("Company").is_ok());
    assert!(schema.validate_node_type("RandomType123").is_ok());
}

#[test]
fn test_schemaless_accepts_any_edge_type() {
    let schema = GraphSchema::schemaless();

    // Schemaless mode should accept any edge type
    assert!(schema
        .validate_edge_type("WORKS_AT", "Person", "Company")
        .is_ok());
    assert!(schema
        .validate_edge_type("RANDOM_EDGE", "Foo", "Bar")
        .is_ok());
}

// =============================================================================
// AC-3: Validation de schéma strict
// =============================================================================

#[test]
fn test_strict_schema_rejects_invalid_node_type() {
    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person"))
        .with_node_type(NodeType::new("Company"));

    // Valid types should pass
    assert!(schema.validate_node_type("Person").is_ok());
    assert!(schema.validate_node_type("Company").is_ok());

    // Invalid type should be rejected
    let result = schema.validate_node_type("Animal");
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(err.to_string().contains("Animal"));
    assert!(err.to_string().contains("Person") || err.to_string().contains("Company"));
}

#[test]
fn test_strict_schema_rejects_invalid_edge_type() {
    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person"))
        .with_node_type(NodeType::new("Company"))
        .with_edge_type(EdgeType::new("WORKS_AT", "Person", "Company"));

    // Valid edge should pass
    assert!(schema
        .validate_edge_type("WORKS_AT", "Person", "Company")
        .is_ok());

    // Invalid edge type name
    let result = schema.validate_edge_type("OWNS", "Person", "Company");
    assert!(result.is_err());

    // Invalid source node type
    let result = schema.validate_edge_type("WORKS_AT", "Animal", "Company");
    assert!(result.is_err());

    // Invalid target node type
    let result = schema.validate_edge_type("WORKS_AT", "Person", "Animal");
    assert!(result.is_err());
}

#[test]
fn test_schema_validation_error_message_includes_allowed_types() {
    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person"))
        .with_node_type(NodeType::new("Company"));

    let err = schema.validate_node_type("InvalidType").unwrap_err();
    let msg = err.to_string();

    // Error message should list allowed types
    assert!(msg.contains("InvalidType"));
    // Should mention at least one valid type
    assert!(msg.contains("Person") || msg.contains("Company"));
}

// =============================================================================
// Serialization tests
// =============================================================================

#[test]
fn test_graph_schema_serialization_roundtrip() {
    let mut props = HashMap::new();
    props.insert("name".to_string(), ValueType::String);

    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person").with_properties(props))
        .with_edge_type(EdgeType::new("KNOWS", "Person", "Person"));

    let json = serde_json::to_string(&schema).expect("serialization failed");
    let deserialized: GraphSchema = serde_json::from_str(&json).expect("deserialization failed");

    assert_eq!(schema.node_types().len(), deserialized.node_types().len());
    assert_eq!(schema.edge_types().len(), deserialized.edge_types().len());
    assert!(deserialized.has_node_type("Person"));
    assert!(deserialized.has_edge_type("KNOWS"));
}

#[test]
fn test_value_type_serialization() {
    let types = vec![
        ValueType::String,
        ValueType::Integer,
        ValueType::Float,
        ValueType::Boolean,
        ValueType::Vector,
    ];

    for value_type in types {
        let json = serde_json::to_string(&value_type).expect("serialization failed");
        let deserialized: ValueType = serde_json::from_str(&json).expect("deserialization failed");
        assert_eq!(value_type, deserialized);
    }
}

// =============================================================================
// Edge cases
// =============================================================================

#[test]
fn test_empty_schema_is_not_schemaless() {
    // A schema created with new() but no types added is NOT schemaless
    // It's a strict schema that allows nothing
    let schema = GraphSchema::new();

    assert!(!schema.is_schemaless());
    assert!(schema.validate_node_type("Person").is_err());
}

#[test]
fn test_node_type_name_case_sensitive() {
    let schema = GraphSchema::new().with_node_type(NodeType::new("Person"));

    assert!(schema.has_node_type("Person"));
    assert!(!schema.has_node_type("person"));
    assert!(!schema.has_node_type("PERSON"));
}
