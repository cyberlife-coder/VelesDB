//! `VectorCollection`: newtype wrapper around `Collection` for vector workloads.
//!
//! This type provides a stable, typed API for vector collections.
//! Internally it delegates 100% to the `Collection` executor to avoid
//! any data synchronisation issues between separate storage layers.

mod accessors;
mod crud;
mod lifecycle;
mod search;
#[cfg(test)]
mod search_tests;

use crate::collection::types::Collection;

/// A vector collection combining HNSW search, payload storage, and full-text search.
///
/// `VectorCollection` is a typed newtype over `Collection` that provides
/// a stable public API for vector workloads. All storage operations delegate
/// to the single `inner: Collection` instance — no dual-storage desync.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::{VectorCollection, DistanceMetric, Point, StorageMode};
/// use serde_json::json;
///
/// let coll = VectorCollection::create(
///     "./data/docs".into(),
///     "docs",
///     768,
///     DistanceMetric::Cosine,
///     StorageMode::Full,
/// )?;
///
/// coll.upsert(vec![
///     Point::new(1, vec![0.1; 768], Some(json!({"title": "Hello"}))),
/// ])?;
///
/// let results = coll.search(&vec![0.1; 768], 10)?;
/// # Ok::<(), velesdb_core::Error>(())
/// ```
#[derive(Clone)]
pub struct VectorCollection {
    /// Single source of truth — all operations delegate here.
    pub(crate) inner: Collection,
}
