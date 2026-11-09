//! Typed collection resolver for CLI commands.
//!
//! Provides type-safe dispatch over the three collection kinds
//! (Vector, Graph, Metadata) so that CLI commands can display
//! type-specific details and call the correct APIs.

use velesdb_core::{
    AnyCollection, Database, GraphCollection, MetadataCollection, VectorCollection,
};

/// A typed handle to one of the three collection kinds.
pub enum TypedCollection {
    Vector(VectorCollection),
    Graph(GraphCollection),
    Metadata(MetadataCollection),
}

/// Resolves a collection name to its typed variant.
///
/// Delegates to core's [`Database::get_any_collection`], the one registry
/// walk (Vector → Graph → Metadata) every surface shares. `AnyCollection` is
/// `#[non_exhaustive]`: a variant core adds later maps to `None` here —
/// matching the prior behaviour for a kind the CLI cannot dispatch — rather
/// than being silently reachable.
///
/// Returns `None` if the collection does not exist in any registry.
#[must_use]
pub fn resolve_collection(db: &Database, name: &str) -> Option<TypedCollection> {
    match db.get_any_collection(name)? {
        AnyCollection::Vector(c) => Some(TypedCollection::Vector(c)),
        AnyCollection::Graph(c) => Some(TypedCollection::Graph(c)),
        AnyCollection::Metadata(c) => Some(TypedCollection::Metadata(c)),
        _ => None,
    }
}

/// Returns a human-readable label for the collection type.
///
/// Falls back to `"unknown"` if the collection is not found in any typed registry.
#[must_use]
pub fn collection_type_label(db: &Database, name: &str) -> &'static str {
    match db.get_any_collection(name) {
        Some(AnyCollection::Vector(_)) => "Vector",
        Some(AnyCollection::Graph(_)) => "Graph",
        Some(AnyCollection::Metadata(_)) => "Metadata",
        _ => "unknown",
    }
}

#[cfg(test)]
#[path = "collection_helpers_tests.rs"]
mod collection_helpers_tests;
