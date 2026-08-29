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
#[allow(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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
#[allow(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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
#[allow(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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
        assert!(db.get_vector_collection("test_coll").is_some());
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
