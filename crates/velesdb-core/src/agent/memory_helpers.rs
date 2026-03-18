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
