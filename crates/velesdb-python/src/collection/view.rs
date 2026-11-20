//! A Python `Collection` built from core's `AnyCollection`, and the reads
//! core dispatches by kind, so a graph counts and reads every stored node as
//! core defines them.

use std::sync::Arc;

use velesdb_core::{AnyCollection, Database as CoreDatabase};

use super::{Collection, CollectionKind};

impl Collection {
    /// Create a wrapper for any collection, its kind read from the variant —
    /// a graph or metadata collection behind its vector view, so vector-only
    /// methods fail loud rather than returning empty results (F2.2). `None`
    /// for a kind this view does not know (`AnyCollection` is
    /// `#[non_exhaustive]`).
    pub(crate) fn from_any(
        any: AnyCollection,
        db: Arc<CoreDatabase>,
        name: String,
    ) -> Option<Self> {
        let (inner, kind) = match &any {
            AnyCollection::Vector(v) => (v.clone(), CollectionKind::Vector),
            AnyCollection::Graph(g) => (g.clone().into_vector_view(), CollectionKind::Graph),
            AnyCollection::Metadata(m) => (m.clone().into_vector_view(), CollectionKind::Metadata),
            _ => return None,
        };
        Some(Self {
            inner,
            any,
            db,
            name,
            kind,
        })
    }

    /// The collection's size: core's `AnyCollection::point_count`.
    pub(super) fn size(&self) -> usize {
        self.any.point_count()
    }

    /// The points `ids` name: core's `AnyCollection::get` (a graph's node
    /// without an embedding reads with an empty `vector`). `get` and `in`
    /// read here.
    pub(super) fn read_points(&self, ids: &[u64]) -> Vec<Option<velesdb_core::Point>> {
        self.any.get(ids)
    }
}
