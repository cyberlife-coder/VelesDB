//! Typed collection resolver for CLI commands.
//!
//! Provides type-safe dispatch over the three collection kinds
//! (Vector, Graph, Metadata) so that CLI commands can display
//! type-specific details and call the correct APIs.

use velesdb_core::{Database, GraphCollection, MetadataCollection, VectorCollection};

/// A typed handle to one of the three collection kinds.
pub enum TypedCollection {
    Vector(VectorCollection),
    Graph(GraphCollection),
    Metadata(MetadataCollection),
}

/// Resolves a collection name to its typed variant.
///
/// Checks typed registries in order: Vector → Graph → Metadata.
/// Returns `None` if the collection does not exist in any registry.
#[must_use]
pub fn resolve_collection(db: &Database, name: &str) -> Option<TypedCollection> {
    if let Some(c) = db.get_vector_collection(name) {
        return Some(TypedCollection::Vector(c));
    }
    if let Some(c) = db.get_graph_collection(name) {
        return Some(TypedCollection::Graph(c));
    }
    if let Some(c) = db.get_metadata_collection(name) {
        return Some(TypedCollection::Metadata(c));
    }
    None
}

/// Returns a human-readable label for the collection type.
///
/// Falls back to `"unknown"` if the collection is not found in any typed registry.
#[must_use]
pub fn collection_type_label(db: &Database, name: &str) -> &'static str {
    if db.get_vector_collection(name).is_some() {
        return "Vector";
    }
    if db.get_graph_collection(name).is_some() {
        return "Graph";
    }
    if db.get_metadata_collection(name).is_some() {
        return "Metadata";
    }
    "unknown"
}

#[cfg(test)]
#[path = "collection_helpers_tests.rs"]
mod collection_helpers_tests;
