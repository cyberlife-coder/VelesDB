//! Shared helpers for agent memory subsystems (EPIC-010).
//!
//! Extracts common patterns used by `SemanticMemory`, `EpisodicMemory`, and
//! `ProceduralMemory` to avoid code duplication across the three modules.
//!
//! These helpers are ready for adoption by memory submodules.
//! Currently tested directly; callers will migrate in a follow-up.

use crate::{Database, DistanceMetric, Point};
use parking_lot::RwLock;
use std::collections::HashSet;

use super::error::AgentMemoryError;

/// Looks up a legacy `Collection` by name, returning an `AgentMemoryError` if absent.
#[allow(deprecated)]
pub(super) fn get_collection(
    db: &Database,
    name: &str,
) -> Result<crate::Collection, AgentMemoryError> {
    db.get_collection(name)
        .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))
}

/// Validates that `actual` matches the `expected` embedding dimension.
pub(super) fn validate_dimension(expected: usize, actual: usize) -> Result<(), AgentMemoryError> {
    if actual != expected {
        return Err(AgentMemoryError::DimensionMismatch { expected, actual });
    }
    Ok(())
}

/// Opens an existing collection or creates a new one with cosine distance.
///
/// If the collection already exists, verifies that dimensions match and returns
/// the existing dimension. If it does not exist, creates it with `dimension`.
#[allow(deprecated)]
pub(super) fn open_or_create_collection(
    db: &Database,
    collection_name: &str,
    dimension: usize,
) -> Result<usize, AgentMemoryError> {
    if let Some(collection) = db.get_collection(collection_name) {
        let existing_dim = collection.config().dimension;
        if existing_dim != dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: existing_dim,
                actual: dimension,
            });
        }
        Ok(existing_dim)
    } else {
        db.create_collection(collection_name, dimension, DistanceMetric::Cosine)?;
        Ok(dimension)
    }
}

/// Loads all IDs from an existing collection into a `HashSet`.
#[allow(deprecated)]
pub(super) fn load_stored_ids(db: &Database, collection_name: &str) -> HashSet<u64> {
    db.get_collection(collection_name)
        .map(|c| c.all_ids().into_iter().collect())
        .unwrap_or_default()
}

/// Removes all existing points from a collection.
#[allow(deprecated)]
pub(super) fn clear_collection(collection: &crate::Collection) -> Result<(), AgentMemoryError> {
    let existing_ids = collection.all_ids();
    if !existing_ids.is_empty() {
        collection
            .delete(&existing_ids)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
    }
    Ok(())
}

/// Clears and rebuilds `stored_ids` from a set of deserialized points.
pub(super) fn rebuild_stored_ids(stored_ids: &RwLock<HashSet<u64>>, points: &[Point]) {
    let mut ids = stored_ids.write();
    ids.clear();
    for point in points {
        ids.insert(point.id);
    }
}

/// Serializes points from a collection using the given ID set.
#[allow(deprecated)]
pub(super) fn serialize_points(
    collection: &crate::Collection,
    ids: &[u64],
) -> Result<Vec<u8>, AgentMemoryError> {
    let points: Vec<_> = collection.get(ids).into_iter().flatten().collect();
    serde_json::to_vec(&points).map_err(|e| AgentMemoryError::IoError(e.to_string()))
}

/// Deserializes points from bytes and replaces the collection contents.
///
/// Returns the deserialized points so callers can rebuild their own indexes.
#[allow(deprecated)]
pub(super) fn deserialize_into_collection(
    data: &[u8],
    collection: &crate::Collection,
) -> Result<Option<Vec<Point>>, AgentMemoryError> {
    if data.is_empty() {
        return Ok(None);
    }

    let points: Vec<Point> =
        serde_json::from_slice(data).map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

    clear_collection(collection)?;
    upsert_points(collection, points.clone())?;

    Ok(Some(points))
}

/// Deletes points by ID from a collection.
#[allow(deprecated)]
pub(super) fn delete_from_collection(
    collection: &crate::Collection,
    ids: &[u64],
) -> Result<(), AgentMemoryError> {
    collection
        .delete(ids)
        .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))
}

/// Upserts points into a collection.
#[allow(deprecated)]
pub(super) fn upsert_points(
    collection: &crate::Collection,
    points: Vec<Point>,
) -> Result<(), AgentMemoryError> {
    collection
        .upsert(points)
        .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))
}

/// Searches a collection by vector similarity.
#[allow(deprecated)]
pub(super) fn search_collection(
    collection: &crate::Collection,
    query: &[f32],
    k: usize,
) -> Result<Vec<crate::SearchResult>, AgentMemoryError> {
    collection
        .search(query, k)
        .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))
}

/// Deletes a point by ID, removes it from the `stored_ids` tracking set, and
/// clears its TTL entry.
///
/// This is the common delete pattern shared by `SemanticMemory` and
/// `ProceduralMemory`. `EpisodicMemory` has additional temporal-index cleanup.
#[allow(deprecated)]
pub(super) fn delete_tracked_point(
    db: &Database,
    collection_name: &str,
    id: u64,
    stored_ids: &RwLock<HashSet<u64>>,
    ttl: &super::ttl::MemoryTtl,
) -> Result<(), AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    delete_from_collection(&collection, &[id])?;
    stored_ids.write().remove(&id);
    ttl.remove(id);
    Ok(())
}

/// Serializes all tracked points from a collection using the `stored_ids` set.
///
/// Shared by `SemanticMemory` and `ProceduralMemory`.
/// `EpisodicMemory` uses temporal-index IDs instead.
#[allow(deprecated)]
pub(super) fn serialize_tracked_points(
    db: &Database,
    collection_name: &str,
    stored_ids: &RwLock<HashSet<u64>>,
) -> Result<Vec<u8>, AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    let all_ids: Vec<u64> = stored_ids.read().iter().copied().collect();
    serialize_points(&collection, &all_ids)
}

/// Replaces collection contents from serialized bytes and rebuilds the
/// `stored_ids` tracking set.
///
/// Shared by `SemanticMemory` and `ProceduralMemory`.
/// `EpisodicMemory` rebuilds its temporal index instead.
#[allow(deprecated)]
pub(super) fn deserialize_tracked_points(
    db: &Database,
    collection_name: &str,
    data: &[u8],
    stored_ids: &RwLock<HashSet<u64>>,
) -> Result<(), AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    if let Some(points) = deserialize_into_collection(data, &collection)? {
        rebuild_stored_ids(stored_ids, &points);
    }
    Ok(())
}

/// Validates the query embedding, searches the collection, and filters out
/// expired results.
///
/// This is the common search preamble shared by `SemanticMemory::query`,
/// `EpisodicMemory::recall_similar`, and `ProceduralMemory::recall`. Each
/// caller then maps the returned `SearchResult` items into its own return
/// type.
#[allow(deprecated)]
pub(super) fn search_filtered(
    db: &Database,
    collection_name: &str,
    dimension: usize,
    query_embedding: &[f32],
    k: usize,
    ttl: &super::ttl::MemoryTtl,
) -> Result<Vec<crate::SearchResult>, AgentMemoryError> {
    validate_dimension(dimension, query_embedding.len())?;
    let collection = get_collection(db, collection_name)?;
    let results = search_collection(&collection, query_embedding, k)?;
    Ok(results
        .into_iter()
        .filter(|r| !ttl.is_expired(r.point.id))
        .collect())
}

/// Validates a count-prefixed binary buffer and returns the entry count.
///
/// The expected format is `[count: u64 LE][entries: count * entry_size bytes]`.
/// Returns `None` if the buffer is too small, the count cannot be read, or the
/// total length does not match the declared count.
///
/// Used by `TemporalIndex::deserialize` (16-byte entries) and
/// `MemoryTtl::deserialize` (24-byte entries).
#[allow(clippy::cast_possible_truncation)] // count validated against buffer length
pub(super) fn validate_binary_header(data: &[u8], entry_size: usize) -> Option<usize> {
    if data.len() < 8 {
        return None;
    }
    let count = u64::from_le_bytes(data[0..8].try_into().ok()?) as usize;
    if data.len() != 8 + count * entry_size {
        return None;
    }
    Some(count)
}

/// Initializes the common fields shared by `SemanticMemory` and `ProceduralMemory`.
///
/// Opens or creates the backing collection, resolves the actual dimension,
/// and loads the set of stored point IDs. Returns a tuple of
/// `(collection_name, actual_dimension, stored_ids)` ready for struct
/// construction.
pub(super) fn init_tracked_memory(
    db: &Database,
    collection_name: &str,
    dimension: usize,
) -> Result<(String, usize, RwLock<HashSet<u64>>), AgentMemoryError> {
    let name = collection_name.to_string();
    let actual_dimension = open_or_create_collection(db, &name, dimension)?;
    let stored_ids = RwLock::new(load_stored_ids(db, &name));
    Ok((name, actual_dimension, stored_ids))
}

/// Validates an optional embedding dimension and returns a concrete vector.
///
/// If `embedding` is `Some`, validates that its length matches `dimension`
/// and returns a clone. If `None`, returns a zero-vector of the given
/// dimension. This is the common setup shared by `EpisodicMemory::record`
/// and `ProceduralMemory::learn`.
pub(super) fn resolve_embedding(
    dimension: usize,
    embedding: Option<&[f32]>,
) -> Result<Vec<f32>, AgentMemoryError> {
    if let Some(emb) = embedding {
        validate_dimension(dimension, emb.len())?;
    }
    Ok(embedding.map_or_else(|| vec![0.0; dimension], <[f32]>::to_vec))
}

/// Executes a `VelesQL` query string against a named collection.
///
/// Resolves the collection from the database by name, then delegates to
/// `Collection::execute_query_str`.
///
/// # Errors
///
/// Returns `AgentMemoryError::CollectionError` if the collection is not found,
/// or `AgentMemoryError::DatabaseError` if the query fails to parse or execute.
#[allow(deprecated)]
pub(super) fn execute_velesql(
    db: &Database,
    collection_name: &str,
    sql: &str,
    params: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<Vec<crate::SearchResult>, AgentMemoryError> {
    let collection = get_collection(db, collection_name)?;
    collection
        .execute_query_str(sql, params)
        .map_err(AgentMemoryError::DatabaseError)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_dimension ---

    #[test]
    fn validate_dimension_matching_returns_ok() {
        assert!(validate_dimension(128, 128).is_ok());
    }

    #[test]
    fn validate_dimension_zero_matches_zero() {
        assert!(validate_dimension(0, 0).is_ok());
    }

    #[test]
    fn validate_dimension_mismatch_returns_error() {
        let err = validate_dimension(128, 64).unwrap_err();
        assert!(
            matches!(
                err,
                AgentMemoryError::DimensionMismatch {
                    expected: 128,
                    actual: 64
                }
            ),
            "Expected DimensionMismatch, got: {err:?}"
        );
    }

    #[test]
    fn validate_dimension_swapped_values_are_distinct() {
        // validate_dimension(64, 128) should give expected=64, actual=128
        let err = validate_dimension(64, 128).unwrap_err();
        assert!(matches!(
            err,
            AgentMemoryError::DimensionMismatch {
                expected: 64,
                actual: 128
            }
        ));
    }

    // --- rebuild_stored_ids ---

    #[test]
    fn rebuild_stored_ids_populates_from_points() {
        let stored_ids = RwLock::new(HashSet::new());
        let points = vec![
            Point::without_payload(10, vec![0.0; 4]),
            Point::without_payload(20, vec![0.0; 4]),
            Point::without_payload(30, vec![0.0; 4]),
        ];

        rebuild_stored_ids(&stored_ids, &points);

        let ids = stored_ids.read();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&10));
        assert!(ids.contains(&20));
        assert!(ids.contains(&30));
    }

    #[test]
    fn rebuild_stored_ids_clears_previous_ids() {
        let mut initial = HashSet::new();
        initial.insert(1);
        initial.insert(2);
        let stored_ids = RwLock::new(initial);

        let points = vec![Point::without_payload(99, vec![0.0; 4])];
        rebuild_stored_ids(&stored_ids, &points);

        let ids = stored_ids.read();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&99));
        assert!(!ids.contains(&1));
        assert!(!ids.contains(&2));
    }

    #[test]
    fn rebuild_stored_ids_empty_points_clears_all() {
        let mut initial = HashSet::new();
        initial.insert(5);
        let stored_ids = RwLock::new(initial);

        rebuild_stored_ids(&stored_ids, &[]);

        assert!(stored_ids.read().is_empty());
    }

    #[test]
    fn rebuild_stored_ids_deduplicates() {
        let stored_ids = RwLock::new(HashSet::new());
        let points = vec![
            Point::without_payload(1, vec![0.0; 4]),
            Point::without_payload(1, vec![1.0; 4]), // same ID
        ];

        rebuild_stored_ids(&stored_ids, &points);

        let ids = stored_ids.read();
        assert_eq!(ids.len(), 1);
        assert!(ids.contains(&1));
    }

    // --- open_or_create_collection (requires persistence + tempdir) ---

    #[cfg(feature = "persistence")]
    #[allow(deprecated)]
    mod persistence_tests {
        use super::*;
        use tempfile::TempDir;

        #[test]
        fn open_or_create_creates_new_collection() {
            let tmp = TempDir::new().unwrap();
            let db = Database::open(tmp.path()).unwrap();

            let dim = open_or_create_collection(&db, "test_coll", 64).unwrap();
            assert_eq!(dim, 64);

            // Collection should now be retrievable.
            assert!(db.get_collection("test_coll").is_some());
        }

        #[test]
        fn open_or_create_returns_existing_with_matching_dim() {
            let tmp = TempDir::new().unwrap();
            let db = Database::open(tmp.path()).unwrap();

            // First call creates.
            open_or_create_collection(&db, "my_coll", 128).unwrap();

            // Second call with same dim should succeed.
            let dim = open_or_create_collection(&db, "my_coll", 128).unwrap();
            assert_eq!(dim, 128);
        }

        #[test]
        fn open_or_create_errors_on_dimension_mismatch() {
            let tmp = TempDir::new().unwrap();
            let db = Database::open(tmp.path()).unwrap();

            open_or_create_collection(&db, "dim_coll", 64).unwrap();

            let err = open_or_create_collection(&db, "dim_coll", 128).unwrap_err();
            assert!(
                matches!(
                    err,
                    AgentMemoryError::DimensionMismatch {
                        expected: 64,
                        actual: 128
                    }
                ),
                "Expected DimensionMismatch, got: {err:?}"
            );
        }
    }
}
