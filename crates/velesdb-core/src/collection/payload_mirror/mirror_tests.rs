//! Integration tests: payload mirror behavior through the Collection API.
//!
//! Validates the adaptive build (scan-debt trigger), result parity with the
//! sequential JSON scan path, incremental maintenance on upsert/delete, and
//! the invalidation safety net.

#![allow(clippy::cast_precision_loss)]

use crate::collection::payload_mirror::MIRROR_MIN_ROWS;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::collections::HashMap;
use tempfile::TempDir;

const ROWS: usize = 300;

fn make_point(i: usize) -> Point {
    Point {
        id: i as u64,
        vector: vec![i as f32 / ROWS as f32, 0.5, 0.25, 1.0],
        payload: Some(serde_json::json!({
            "category": format!("cat{}", i % 3),
            "price": i,
            "active": i.is_multiple_of(2),
        })),
        sparse_vectors: None,
    }
}

fn setup_collection() -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("mirror_col");
    let col = Collection::create(path, 4, DistanceMetric::Cosine).expect("create collection");
    col.upsert((0..ROWS).map(make_point)).expect("upsert");
    (dir, col)
}

fn query_ids(col: &Collection, query: &str) -> Vec<u64> {
    let params = HashMap::new();
    let mut ids: Vec<u64> = col
        .execute_query_str(query, &params)
        .expect("query succeeds")
        .into_iter()
        .map(|r| r.point.id)
        .collect();
    ids.sort_unstable();
    ids
}

fn mirror_built(col: &Collection) -> bool {
    col.payload_mirror.state.read().is_some()
}

#[test]
fn test_mirror_builds_after_scan_debt_and_preserves_results() {
    let (_dir, col) = setup_collection();
    let query = "SELECT * FROM c WHERE category = 'cat1' AND price >= 30 LIMIT 500";

    // First query takes the sequential scan path and accrues scan debt.
    let scan_ids = query_ids(&col, query);
    assert!(!mirror_built(&col), "mirror must not build on first scan");
    assert!(col.payload_mirror.scan_debt() >= ROWS as u64);

    // Second query crosses the debt threshold, builds the mirror, and must
    // return exactly the same rows.
    let mirror_ids = query_ids(&col, query);
    assert!(mirror_built(&col), "mirror builds once debt covers a scan");
    assert_eq!(scan_ids, mirror_ids);

    let expected: Vec<u64> = (0..ROWS as u64)
        .filter(|i| i % 3 == 1 && *i >= 30)
        .collect();
    assert_eq!(mirror_ids, expected);
}

#[test]
fn test_mirror_parity_across_operators() {
    let (_dir, col) = setup_collection();
    let queries = [
        "SELECT * FROM c WHERE price BETWEEN 10 AND 20 LIMIT 500",
        "SELECT * FROM c WHERE category != 'cat0' AND price < 12 LIMIT 500",
        "SELECT * FROM c WHERE category IN ('cat0', 'cat2') AND price <= 9 LIMIT 500",
        "SELECT * FROM c WHERE active = true AND price > 290 LIMIT 500",
        "SELECT * FROM c WHERE category = 'cat1' OR price >= 297 LIMIT 500",
        "SELECT * FROM c WHERE NOT (category = 'cat1') AND price < 7 LIMIT 500",
    ];

    // Scan-path ground truth (mirror not yet built).
    let ground_truth: Vec<Vec<u64>> = queries.iter().map(|q| query_ids(&col, q)).collect();

    // Force the mirror and re-run every query.
    col.build_payload_mirror();
    assert!(mirror_built(&col));
    for (query, expected) in queries.iter().zip(&ground_truth) {
        let got = query_ids(&col, query);
        assert_eq!(&got, expected, "mirror parity for {query}");
    }
}

#[test]
fn test_mirror_stays_in_sync_after_upsert_and_delete() {
    let (_dir, col) = setup_collection();
    col.build_payload_mirror();
    let query = "SELECT * FROM c WHERE category = 'fresh' LIMIT 500";
    assert!(query_ids(&col, query).is_empty());

    // Upsert a new point and retag an existing one.
    let mut fresh = make_point(1000);
    fresh.payload = Some(serde_json::json!({"category": "fresh", "price": 1}));
    let mut retagged = make_point(5);
    retagged.payload = Some(serde_json::json!({"category": "fresh", "price": 2}));
    col.upsert(vec![fresh, retagged]).expect("upsert");

    assert!(mirror_built(&col), "incremental hook keeps the mirror warm");
    assert_eq!(query_ids(&col, query), vec![5, 1000]);

    // Delete one of them; the tombstone must hide it immediately.
    col.delete(&[5]).expect("delete");
    assert!(mirror_built(&col));
    assert_eq!(query_ids(&col, query), vec![1000]);

    // The retagged point must no longer match its old category.
    let old = query_ids(
        &col,
        "SELECT * FROM c WHERE category = 'cat2' AND price = 5 LIMIT 10",
    );
    assert!(old.is_empty());
}

/// Returns `(row_count, deleted_row_count)` of the mirror's `ColumnStore`.
fn mirror_store_counts(col: &Collection) -> (usize, usize) {
    let guard = col.payload_mirror.state.read();
    let state = guard.as_ref().expect("mirror must be built");
    (state.store.row_count(), state.store.deleted_row_count())
}

/// GIVEN a built mirror over 300 rows
/// WHEN a delete batch pushes the dead-row ratio past the PostgreSQL-inspired
///      auto-vacuum threshold (20% AND >= 50 dead rows)
/// THEN the store is compacted synchronously (tombstones removed) and the
///      remapped mirror keeps answering filters correctly.
#[test]
fn test_auto_vacuum_triggers_on_delete_threshold() {
    let (_dir, col) = setup_collection();
    col.build_payload_mirror();

    // 75 of 300 rows = 25% dead ratio, >= 50 dead rows: trigger fires.
    let ids: Vec<u64> = (0..75).collect();
    col.delete(&ids).expect("delete");

    assert!(mirror_built(&col), "mirror must survive the vacuum");
    let (rows, dead) = mirror_store_counts(&col);
    assert_eq!(dead, 0, "auto-vacuum must compact the tombstones");
    assert_eq!(rows, ROWS - 75, "only live rows survive the compaction");

    // Row indices were remapped — columnar answers must stay exact.
    let expected: Vec<u64> = (75..ROWS as u64).filter(|i| i % 3 == 1).collect();
    assert_eq!(
        query_ids(&col, "SELECT * FROM c WHERE category = 'cat1' LIMIT 500"),
        expected
    );
}

/// Below `min_dead_rows` (50) the trigger must NOT fire: tombstones remain
/// until the threshold is crossed.
#[test]
fn test_auto_vacuum_stays_idle_below_threshold() {
    let (_dir, col) = setup_collection();
    col.build_payload_mirror();

    let ids: Vec<u64> = (0..49).collect();
    col.delete(&ids).expect("delete");

    let (rows, dead) = mirror_store_counts(&col);
    assert_eq!(dead, 49, "below min_dead_rows: tombstones must remain");
    assert_eq!(rows, ROWS, "no compaction below the threshold");
}

#[test]
fn test_unsupported_conditions_fall_back_with_correct_results() {
    let (_dir, col) = setup_collection();
    col.build_payload_mirror();

    // LIKE is not columnar-eligible — must fall back and stay correct.
    let ids = query_ids(
        &col,
        "SELECT * FROM c WHERE category LIKE 'cat1%' AND price < 10 LIMIT 500",
    );
    let expected: Vec<u64> = (0..10u64).filter(|i| i % 3 == 1).collect();
    assert_eq!(ids, expected);
}

#[test]
fn test_small_collections_never_build_a_mirror() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("small_col");
    let col = Collection::create(path, 4, DistanceMetric::Cosine).expect("create collection");
    let small = MIRROR_MIN_ROWS / 2;
    col.upsert((0..small).map(make_point)).expect("upsert");

    let query = "SELECT * FROM c WHERE category = 'cat1' LIMIT 500";
    for _ in 0..4 {
        let ids = query_ids(&col, query);
        assert_eq!(ids.len(), small.div_ceil(3));
    }
    assert!(!mirror_built(&col), "below MIRROR_MIN_ROWS, never build");
}

#[test]
fn test_mixed_type_field_falls_back_with_correct_results() {
    let (_dir, col) = setup_collection();
    // Poison "price" with a string value on one row.
    let mut odd = make_point(42);
    odd.payload = Some(serde_json::json!({"category": "cat0", "price": "n/a"}));
    col.upsert(vec![odd]).expect("upsert");
    col.build_payload_mirror();

    // Numeric Eq on the mixed-type field: the mirror answers from the Float
    // column (exact for numeric rows); the string row can never match a
    // numeric literal in JSON semantics either.
    let ids = query_ids(&col, "SELECT * FROM c WHERE price = 41 LIMIT 10");
    assert_eq!(ids, vec![41]);

    // String Eq must fall back (ineligible) and still find the odd row.
    let ids = query_ids(&col, "SELECT * FROM c WHERE price = 'n/a' LIMIT 10");
    assert_eq!(ids, vec![42]);
}

/// GIVEN a built mirror and a writer thread applying upsert batches where
///       every point in batch `g` carries `gen == g`
/// WHEN a reader thread concurrently probes `candidate_ids` for each gen
/// THEN every observed candidate set is all-or-nothing (0 or the full batch):
///      `apply_upserts` holds one write lock for the whole batch, so filtered
///      queries can never see a half-applied batch.
#[test]
fn test_apply_upserts_batch_is_atomic_for_concurrent_readers() {
    use crate::collection::payload_mirror::MirrorAnswer;
    use std::sync::atomic::{AtomicBool, Ordering};

    const BATCH: usize = 50;
    const GENS: u64 = 20;

    let (_dir, col) = setup_collection();
    col.build_payload_mirror();
    assert!(mirror_built(&col));

    let gen_eq = |gen: u64| crate::filter::Condition::Eq {
        field: "gen".to_string(),
        value: serde_json::json!(gen),
    };
    let done = AtomicBool::new(false);

    std::thread::scope(|s| {
        let mirror = &col.payload_mirror;
        let done_ref = &done;
        s.spawn(move || {
            for gen in 1..=GENS {
                let points: Vec<Point> = (0..BATCH)
                    .map(|i| Point {
                        id: i as u64,
                        vector: vec![0.0; 4],
                        payload: Some(serde_json::json!({"gen": gen})),
                        sparse_vectors: None,
                    })
                    .collect();
                mirror.apply_upserts(&points);
            }
            done_ref.store(true, Ordering::Release);
        });

        // At least a few full passes, overlapping the writer when possible.
        let mut passes_after_done = 0;
        while passes_after_done < 3 {
            for gen in 1..=GENS {
                if let MirrorAnswer::Ids(ids) = mirror.candidate_ids(&gen_eq(gen)) {
                    assert!(
                        ids.is_empty() || ids.len() == BATCH,
                        "half-applied batch visible: gen {gen} matched {} of {BATCH} rows",
                        ids.len()
                    );
                }
            }
            if done.load(Ordering::Acquire) {
                passes_after_done += 1;
            }
        }
    });

    // Once the writer is done, the last generation must be fully visible.
    match col.payload_mirror.candidate_ids(&gen_eq(GENS)) {
        MirrorAnswer::Ids(ids) => assert_eq!(ids.len(), BATCH),
        _ => panic!("mirror must still be built and answer the Eq probe"),
    }
}

#[test]
fn test_metadata_only_collection_uses_mirror() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("meta_col");
    let col = Collection::create_metadata_only(path, "meta").expect("create collection");
    let points: Vec<Point> = (0..ROWS)
        .map(|i| Point {
            id: i as u64,
            vector: vec![],
            payload: Some(serde_json::json!({"kind": format!("k{}", i % 5), "rank": i})),
            sparse_vectors: None,
        })
        .collect();
    col.upsert(points).expect("upsert");

    let query = "SELECT * FROM c WHERE kind = 'k3' AND rank > 200 LIMIT 500";
    let scan_ids = query_ids(&col, query);
    let _ = query_ids(&col, query); // crosses the debt threshold
    assert!(mirror_built(&col), "metadata-only collections build too");
    let mirror_ids = query_ids(&col, query);
    assert_eq!(scan_ids, mirror_ids);

    // upsert_metadata path keeps the mirror in sync.
    col.upsert(vec![Point {
        id: 9999,
        vector: vec![],
        payload: Some(serde_json::json!({"kind": "k3", "rank": 999})),
        sparse_vectors: None,
    }])
    .expect("upsert");
    assert!(mirror_built(&col));
    let after = query_ids(&col, query);
    assert!(after.contains(&9999));
}
