//! Tests for collection_helpers module (Phase 2.1)

use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, GraphSchema};

use crate::collection_helpers::{collection_type_label, resolve_collection, TypedCollection};

// =============================================================================
// resolve_collection tests
// =============================================================================

#[test]
fn test_resolve_collection_vector() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("vectors", 128, DistanceMetric::Cosine)
        .unwrap();

    let result = resolve_collection(&db, "vectors");
    assert!(result.is_some());
    assert!(matches!(result.unwrap(), TypedCollection::Vector(_)));
}

#[test]
fn test_resolve_collection_graph() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_graph_collection("knowledge", GraphSchema::schemaless())
        .unwrap();

    let result = resolve_collection(&db, "knowledge");
    assert!(result.is_some());
    assert!(matches!(result.unwrap(), TypedCollection::Graph(_)));
}

#[test]
fn test_resolve_collection_metadata() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_metadata_collection("catalog").unwrap();

    let result = resolve_collection(&db, "catalog");
    assert!(result.is_some());
    assert!(matches!(result.unwrap(), TypedCollection::Metadata(_)));
}

#[test]
fn test_resolve_collection_not_found() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();

    let result = resolve_collection(&db, "nonexistent");
    assert!(result.is_none());
}

#[test]
fn test_resolve_collection_multiple_types() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("vec_col", 64, DistanceMetric::Euclidean)
        .unwrap();
    db.create_graph_collection("graph_col", GraphSchema::schemaless())
        .unwrap();
    db.create_metadata_collection("meta_col").unwrap();

    assert!(matches!(
        resolve_collection(&db, "vec_col").unwrap(),
        TypedCollection::Vector(_)
    ));
    assert!(matches!(
        resolve_collection(&db, "graph_col").unwrap(),
        TypedCollection::Graph(_)
    ));
    assert!(matches!(
        resolve_collection(&db, "meta_col").unwrap(),
        TypedCollection::Metadata(_)
    ));
}

// =============================================================================
// collection_type_label tests
// =============================================================================

#[test]
fn test_collection_type_label_vector() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_vector_collection("docs", 768, DistanceMetric::Cosine)
        .unwrap();

    assert_eq!(collection_type_label(&db, "docs"), "Vector");
}

#[test]
fn test_collection_type_label_graph() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_graph_collection("kg", GraphSchema::schemaless())
        .unwrap();

    assert_eq!(collection_type_label(&db, "kg"), "Graph");
}

#[test]
fn test_collection_type_label_metadata() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();
    db.create_metadata_collection("products").unwrap();

    assert_eq!(collection_type_label(&db, "products"), "Metadata");
}

#[test]
fn test_collection_type_label_unknown() {
    let dir = TempDir::new().unwrap();
    let db = Database::open(dir.path()).unwrap();

    assert_eq!(collection_type_label(&db, "nonexistent"), "unknown");
}
