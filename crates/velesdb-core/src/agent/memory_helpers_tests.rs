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
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
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

// --- filter_live_far_end ---

use crate::agent::ttl::{MemoryKind, MemoryTtl};
use crate::collection::graph::GraphEdge;

fn edge(id: u64, source: u64, target: u64) -> GraphEdge {
    GraphEdge::new(id, source, target, "RELATES").expect("non-empty label")
}

#[test]
fn filter_live_far_end_keeps_edges_whose_far_end_is_not_tracked() {
    let ttl = MemoryTtl::new();
    let edges = vec![edge(1, 10, 20), edge(2, 10, 21)];

    let kept = filter_live_far_end(edges.clone(), &ttl, MemoryKind::Semantic, GraphEdge::target);

    assert_eq!(kept, edges);
}

#[test]
fn filter_live_far_end_drops_edges_whose_far_end_expired() {
    let ttl = MemoryTtl::new();
    ttl.set_expiry(MemoryKind::Semantic, 21, 0); // 0 <= now() => expired
    let edges = vec![edge(1, 10, 20), edge(2, 10, 21)];

    let kept = filter_live_far_end(edges, &ttl, MemoryKind::Semantic, GraphEdge::target);

    assert_eq!(kept, vec![edge(1, 10, 20)]);
}

#[test]
fn filter_live_far_end_checks_source_for_incoming_edges() {
    let ttl = MemoryTtl::new();
    ttl.set_expiry(MemoryKind::Semantic, 10, 0); // source of edge 1 expired
    let edges = vec![edge(1, 10, 20), edge(2, 11, 20)];

    let kept = filter_live_far_end(edges, &ttl, MemoryKind::Semantic, GraphEdge::source);

    assert_eq!(kept, vec![edge(2, 11, 20)]);
}

#[test]
fn filter_live_far_end_scopes_expiry_to_the_given_kind() {
    let ttl = MemoryTtl::new();
    // Expired under a different subsystem — must not affect Semantic's view.
    ttl.set_expiry(MemoryKind::Episodic, 20, 0);
    let edges = vec![edge(1, 10, 20)];

    let kept = filter_live_far_end(edges.clone(), &ttl, MemoryKind::Semantic, GraphEdge::target);

    assert_eq!(kept, edges);
}

// --- open_collection (requires persistence + tempdir) ---

#[cfg(feature = "persistence")]
mod persistence_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_or_create_creates_new_collection() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        let dim = open_collection(&db, "test_coll", 64, OnMissing::Create).unwrap();
        assert_eq!(dim, 64);

        // Collection should now be retrievable.
        assert!(db.get_vector_collection("test_coll").is_some());
    }

    #[test]
    fn open_refuses_an_absent_collection_without_creating_it() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        let err = open_collection(&db, "absent", 64, OnMissing::Refuse).unwrap_err();

        assert!(matches!(err, AgentMemoryError::NotFound(_)), "{err:?}");
        assert!(db.get_vector_collection("absent").is_none());
    }

    #[test]
    fn open_or_create_returns_existing_with_matching_dim() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        // First call creates.
        open_collection(&db, "my_coll", 128, OnMissing::Create).unwrap();

        // Second call with same dim should succeed.
        let dim = open_collection(&db, "my_coll", 128, OnMissing::Create).unwrap();
        assert_eq!(dim, 128);
    }

    #[test]
    fn open_or_create_errors_on_dimension_mismatch() {
        let tmp = TempDir::new().unwrap();
        let db = Database::open(tmp.path()).unwrap();

        open_collection(&db, "dim_coll", 64, OnMissing::Create).unwrap();

        let err = open_collection(&db, "dim_coll", 128, OnMissing::Create).unwrap_err();
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
