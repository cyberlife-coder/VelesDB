//! Tests for graph collection creation and retrieval via [`Database`].

#![allow(deprecated)] // Tests use legacy Collection via get_collection().

use super::*;
use crate::collection::{EdgeType, GraphSchema, NodeType};
use crate::DistanceMetric;
use tempfile::tempdir;

// =========================================================================
// create_graph_collection — schemaless
// =========================================================================

#[test]
fn test_create_graph_collection_schemaless() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_graph_collection("kg", GraphSchema::schemaless())
        .unwrap();

    assert!(db.list_collections().contains(&"kg".to_string()));
}

#[test]
fn test_create_graph_collection_with_strict_schema() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let schema = GraphSchema::new()
        .with_node_type(NodeType::new("Person"))
        .with_edge_type(EdgeType::new("KNOWS", "Person", "Person"));

    db.create_graph_collection("social", schema).unwrap();

    let gc = db.get_graph_collection("social").unwrap();
    assert_eq!(gc.name(), "social");
    assert!(!gc.schema().is_schemaless());
    assert!(gc.schema().has_node_type("Person"));
    assert!(gc.schema().has_edge_type("KNOWS"));
}

// =========================================================================
// create_graph_collection_with_embeddings
// =========================================================================

#[test]
fn test_create_graph_collection_with_embeddings() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_graph_collection_with_embeddings(
        "kg_embed",
        GraphSchema::schemaless(),
        128,
        DistanceMetric::Cosine,
    )
    .unwrap();

    let gc = db.get_graph_collection("kg_embed").unwrap();
    assert_eq!(gc.name(), "kg_embed");
    assert!(gc.has_embeddings());
}

// =========================================================================
// create_graph_collection_from_type (pub(super) helper)
// =========================================================================

#[test]
fn test_create_graph_collection_from_type() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let schema = GraphSchema::schemaless();
    db.create_graph_collection_from_type("typed_kg", Some(64), DistanceMetric::Euclidean, &schema)
        .unwrap();

    let gc = db.get_graph_collection("typed_kg").unwrap();
    assert_eq!(gc.name(), "typed_kg");
    assert!(gc.has_embeddings());
}

#[test]
fn test_create_graph_collection_from_type_no_dimension() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let schema = GraphSchema::schemaless();
    db.create_graph_collection_from_type("plain_kg", None, DistanceMetric::Cosine, &schema)
        .unwrap();

    let gc = db.get_graph_collection("plain_kg").unwrap();
    assert!(!gc.has_embeddings());
}

// =========================================================================
// get_graph_collection — Some / None
// =========================================================================

#[test]
fn test_get_graph_collection_returns_none_for_absent() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    assert!(db.get_graph_collection("nonexistent").is_none());
}

#[test]
fn test_get_graph_collection_returns_created() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_graph_collection("g1", GraphSchema::schemaless())
        .unwrap();

    let gc = db.get_graph_collection("g1");
    assert!(gc.is_some());
    assert_eq!(gc.unwrap().name(), "g1");
}

// =========================================================================
// Duplicate name should return error
// =========================================================================

#[test]
fn test_create_duplicate_graph_collection_returns_error() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_graph_collection("dup_graph", GraphSchema::schemaless())
        .unwrap();

    let err = db
        .create_graph_collection("dup_graph", GraphSchema::schemaless())
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

#[test]
fn test_duplicate_name_across_types_graph_vs_vector() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    db.create_graph_collection("shared", GraphSchema::schemaless())
        .unwrap();

    let err = db
        .create_vector_collection("shared", 64, DistanceMetric::Cosine)
        .unwrap_err();
    assert!(matches!(err, crate::Error::CollectionExists(_)));
}

// =========================================================================
// Schema version increments after creation
// =========================================================================

#[test]
fn test_schema_version_increments_on_graph_creation() {
    let dir = tempdir().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let v0 = db.schema_version();

    db.create_graph_collection("g_v1", GraphSchema::schemaless())
        .unwrap();
    assert_eq!(db.schema_version(), v0 + 1);

    db.create_graph_collection_with_embeddings(
        "g_v2",
        GraphSchema::schemaless(),
        32,
        DistanceMetric::Cosine,
    )
    .unwrap();
    assert_eq!(db.schema_version(), v0 + 2);
}
