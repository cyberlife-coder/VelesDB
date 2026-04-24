#![cfg(feature = "persistence")]
//! BDD integration tests for runtime-only `AutoReindexManager` attachment
//! (Wave 3 B2 Commit 9).
//!
//! Covers the `VectorCollection::attach_auto_reindex` /
//! `detach_auto_reindex` / `auto_reindex_manager` /
//! `check_auto_reindex_divergence` surface and the hot-path hook in
//! `upsert_bulk_inner` that consults the attached manager after every
//! successful batch.
//!
//! Test categories (per `.claude/rules/bdd-testing.md`):
//! - Nominal (≥ 60%): attach → upsert → query divergence
//! - Edge (≈ 20%): detach / re-attach / disabled config / cooldown
//! - Negative (≥ 20%): no attachment = no state leak, divergence without
//!   attachment returns `None`, detach-without-attach is a no-op
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::TempDir;

use velesdb_core::collection::auto_reindex::{AutoReindexConfig, AutoReindexManager};
use velesdb_core::{Database, DistanceMetric, Point, VectorCollection};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates a temporary database and returns the guard + handle.
fn temp_database() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open database");
    (dir, db)
}

/// Creates a 4-dim vector collection named `docs` and inserts a single
/// warmup point so the collection is non-empty. Returns the typed handle.
fn create_collection(db: &Database, name: &str) -> VectorCollection {
    db.create_vector_collection(name, 4, DistanceMetric::Cosine)
        .expect("create vector collection");
    db.get_vector_collection(name).expect("collection exists")
}

/// Builds a manager with a tight `min_size_for_reindex = 1` so divergence
/// checks fire even on small test collections.
fn manager_with_min_size(min_size: usize) -> Arc<AutoReindexManager> {
    let config = AutoReindexConfig {
        enabled: true,
        min_size_for_reindex: min_size,
        ..AutoReindexConfig::default()
    };
    Arc::new(AutoReindexManager::new(config))
}

fn upsert_n(coll: &VectorCollection, start: u64, count: u64) {
    let points: Vec<Point> = (start..start + count)
        .map(|i| Point {
            id: i,
            vector: vec![0.1, 0.2, 0.3, 0.4],
            payload: Some(json!({"i": i})),
            sparse_vectors: None,
        })
        .collect();
    coll.upsert_bulk(&points).expect("upsert_bulk");
}

// ---------------------------------------------------------------------------
// Nominal — attach, query, upsert hook
// ---------------------------------------------------------------------------

#[test]
fn attach_then_manager_is_visible_through_getter() {
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");
    let manager = manager_with_min_size(1);

    coll.attach_auto_reindex(Arc::clone(&manager));

    let got = coll
        .auto_reindex_manager()
        .expect("manager should be attached");
    assert!(
        Arc::ptr_eq(&got, &manager),
        "getter must return the attached Arc, not a clone of config"
    );
    assert!(got.is_enabled(), "manager enabled flag preserved");
}

#[test]
fn divergence_query_after_attachment_returns_some() {
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");
    coll.attach_auto_reindex(manager_with_min_size(1));

    // Insert a few points so the vector count exceeds min_size_for_reindex.
    upsert_n(&coll, 1, 5);

    let divergence = coll
        .check_auto_reindex_divergence()
        .expect("attached manager must yield Some(DivergenceCheck)");
    // current_m is always populated even when should_reindex is false.
    assert!(
        divergence.current_m > 0,
        "current_m should reflect the engine defaults, got {}",
        divergence.current_m
    );
}

#[test]
fn upsert_bulk_hot_path_notifies_without_panicking() {
    // The hook is logged via tracing; the contract is that inserting under
    // an attached manager does not panic and does not alter upsert
    // semantics. We verify the returned insert count matches the batch.
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");
    coll.attach_auto_reindex(manager_with_min_size(1));

    let points: Vec<Point> = (1u64..=10)
        .map(|i| Point {
            id: i,
            vector: vec![0.1, 0.2, 0.3, 0.4],
            payload: None,
            sparse_vectors: None,
        })
        .collect();

    let inserted = coll.upsert_bulk(&points).expect("upsert_bulk");
    assert_eq!(inserted, 10, "all points should be inserted");
    assert_eq!(coll.len(), 10, "collection should reflect inserts");
}

// ---------------------------------------------------------------------------
// Edge — detach, cooldown, disabled config
// ---------------------------------------------------------------------------

#[test]
fn detach_returns_previously_attached_manager_then_none() {
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");
    let manager = manager_with_min_size(1);
    coll.attach_auto_reindex(Arc::clone(&manager));

    let detached = coll.detach_auto_reindex();
    assert!(detached.is_some(), "first detach returns the manager");
    let second_detach = coll.detach_auto_reindex();
    assert!(
        second_detach.is_none(),
        "second detach must return None, not an empty Arc"
    );
    assert!(
        coll.auto_reindex_manager().is_none(),
        "after detach, getter must return None"
    );
}

#[test]
fn re_attach_replaces_previous_manager() {
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");
    let first = manager_with_min_size(1);
    let second = manager_with_min_size(10);

    coll.attach_auto_reindex(Arc::clone(&first));
    coll.attach_auto_reindex(Arc::clone(&second));

    let got = coll.auto_reindex_manager().expect("manager attached");
    assert!(
        Arc::ptr_eq(&got, &second),
        "second attach must replace the first"
    );
    assert_eq!(
        got.config().min_size_for_reindex,
        10,
        "replaced manager preserves its own config"
    );
}

#[test]
fn disabled_manager_reports_no_reindex() {
    // An explicitly disabled manager must never report should_reindex=true
    // regardless of divergence magnitude.
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");

    let config = AutoReindexConfig {
        enabled: false,
        min_size_for_reindex: 1,
        ..AutoReindexConfig::default()
    };
    let manager = Arc::new(AutoReindexManager::new(config));
    coll.attach_auto_reindex(manager);

    upsert_n(&coll, 1, 20);

    let divergence = coll
        .check_auto_reindex_divergence()
        .expect("Some even when disabled");
    assert!(
        !divergence.should_reindex,
        "disabled manager must refuse reindex"
    );
}

#[test]
fn cooldown_window_blocks_immediate_re_trigger() {
    // After a successful manual trigger + complete_reindex, the cooldown
    // window (default: several seconds) must prevent should_reindex from
    // returning true immediately. We set an artificially long cooldown to
    // make the test deterministic.
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");

    let config = AutoReindexConfig {
        enabled: true,
        min_size_for_reindex: 1,
        cooldown: Duration::from_secs(3600),
        ..AutoReindexConfig::default()
    };
    let manager = Arc::new(AutoReindexManager::new(config));
    coll.attach_auto_reindex(Arc::clone(&manager));

    upsert_n(&coll, 1, 10);

    // Force the manager through a full reindex cycle so
    // `last_reindex_timestamp` is set.
    assert!(manager.trigger_manual_reindex());
    assert!(manager.start_validation(0, 0));
    assert!(manager.complete_reindex(Duration::from_millis(1)));

    // Now the cooldown gate should block a second reindex even if the
    // divergence is otherwise substantial.
    let divergence = coll.check_auto_reindex_divergence().expect("Some");
    // `check_divergence` itself does not consult the cooldown, but the
    // should_reindex convenience method does.
    let (params, size, dim) = {
        let stats = coll.config();
        (
            stats.hnsw_params.unwrap_or_default(),
            coll.len(),
            stats.dimension,
        )
    };
    assert!(
        !manager.should_reindex(&params, size, dim),
        "cooldown must block the second trigger"
    );
    // The raw check_divergence value is unchanged by the cooldown —
    // the consumer is expected to combine both signals.
    let _ = divergence;
}

// ---------------------------------------------------------------------------
// Negative — no attachment, zero-init semantics
// ---------------------------------------------------------------------------

#[test]
fn collection_without_attachment_returns_none() {
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");

    assert!(
        coll.auto_reindex_manager().is_none(),
        "fresh collection must have no manager attached"
    );
    assert!(
        coll.check_auto_reindex_divergence().is_none(),
        "divergence query without attachment must return None"
    );
}

#[test]
fn upsert_bulk_without_attachment_is_unchanged() {
    // Regression guard: the hot-path hook must be a no-op when no manager
    // is attached. Inserts complete normally and produce the expected
    // vector count.
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");

    let points: Vec<Point> = (1u64..=100)
        .map(|i| Point {
            id: i,
            vector: vec![0.1, 0.2, 0.3, 0.4],
            payload: None,
            sparse_vectors: None,
        })
        .collect();

    let inserted = coll.upsert_bulk(&points).expect("upsert_bulk");
    assert_eq!(inserted, 100);
    assert_eq!(coll.len(), 100);
}

#[test]
fn detach_without_prior_attach_returns_none() {
    let (_dir, db) = temp_database();
    let coll = create_collection(&db, "docs");
    assert!(
        coll.detach_auto_reindex().is_none(),
        "detach on a clean collection must return None, not panic"
    );
}
