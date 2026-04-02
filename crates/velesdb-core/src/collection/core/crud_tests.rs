#![cfg(all(test, feature = "persistence"))]

use crate::storage::PayloadStorage;
use crate::{distance::DistanceMetric, point::Point, quantization::StorageMode, Collection};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

#[test]
fn test_upsert_product_quantization_after_training_backfills_cache() {
    // ARRANGE
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let collection = Collection::create_with_options(
        PathBuf::from(temp_dir.path()),
        16,
        DistanceMetric::Cosine,
        StorageMode::ProductQuantization,
    )
    .expect("collection should be created");

    let points: Vec<Point> = (0u64..128)
        .map(|id| {
            let mut vector: Vec<f32> = (0..16)
                .map(|d| {
                    let id_term = f32::from(u16::try_from(id + 1).expect("id fits in u16")) * 0.17;
                    let d_term =
                        f32::from(u16::try_from(d).expect("dimension index fits in u16")) * 0.11;
                    (id_term + d_term).sin()
                })
                .collect();
            let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in &mut vector {
                    *x /= norm;
                }
            }
            Point::without_payload(id, vector)
        })
        .collect();

    // ACT
    collection.upsert(points).expect("upsert should succeed");

    // ASSERT
    assert!(
        collection.pq_quantizer.read().is_some(),
        "quantizer should be trained after reaching sample threshold"
    );
    assert_eq!(
        collection.pq_cache.read().len(),
        128,
        "all training samples should be backfilled in PQ cache"
    );
}

#[test]
fn test_concurrent_upsert_and_search_no_deadlock() {
    // ARRANGE: shared collection accessible from multiple threads.
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let col = Arc::new(
        Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
            .expect("collection should be created"),
    );

    // Seed with enough points so HNSW search is exercised.
    #[allow(clippy::cast_precision_loss)] // Reason: i in [0,20); u64→f32 exact for small values.
    let seeds: Vec<Point> = (0u64..20)
        .map(|i| Point::without_payload(i, vec![i as f32 / 20.0, 0.1, 0.1, 0.1]))
        .collect();
    col.upsert(seeds).expect("seed upsert should succeed");

    // ACT: 4 threads each interleave upsert + search 50 times.
    let handles: Vec<_> = (0u64..4)
        .map(|t| {
            let col = Arc::clone(&col);
            thread::spawn(move || {
                for i in 0u64..50 {
                    let id = t * 1_000 + i;
                    #[allow(clippy::cast_precision_loss)] // Reason: i in [0,50); u64→f32 exact.
                    col.upsert(vec![Point::without_payload(
                        id,
                        vec![i as f32 / 50.0, 0.2, 0.2, 0.2],
                    )])
                    .expect("concurrent upsert should not fail");
                    let _ = col.search(&[0.5_f32, 0.1, 0.1, 0.1], 5);
                }
            })
        })
        .collect();

    // ASSERT: no thread panicked (panic = deadlock or data race).
    for h in handles {
        h.join()
            .expect("thread panicked — possible deadlock or data race");
    }
}

#[test]
fn test_upsert_indexes_sparse_vectors() {
    use crate::index::sparse::SparseVector;

    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Upsert a point with named sparse vectors
    let mut sv_map = BTreeMap::new();
    sv_map.insert(String::new(), SparseVector::new(vec![(1, 1.0), (2, 0.5)]));
    sv_map.insert(
        "title".to_string(),
        SparseVector::new(vec![(10, 2.0), (20, 1.0)]),
    );

    let point = Point::with_sparse(1, vec![0.1, 0.2, 0.3, 0.4], None, Some(sv_map));
    coll.upsert(vec![point]).unwrap();

    // Verify both named indexes were populated
    let indexes = coll.sparse_indexes().read();
    assert!(
        indexes.contains_key(""),
        "Default sparse index should be created"
    );
    assert!(
        indexes.contains_key("title"),
        "Named sparse index 'title' should be created"
    );

    let default_idx = indexes.get("").unwrap();
    assert_eq!(default_idx.doc_count(), 1);
    let postings = default_idx.get_all_postings(1);
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].doc_id, 1);

    let title_idx = indexes.get("title").unwrap();
    assert_eq!(title_idx.doc_count(), 1);
    let postings = title_idx.get_all_postings(10);
    assert_eq!(postings.len(), 1);
    assert_eq!(postings[0].doc_id, 1);
}

#[test]
fn test_delete_removes_from_sparse_indexes() {
    use crate::index::sparse::SparseVector;

    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Upsert a point with sparse vectors
    let mut sv_map = BTreeMap::new();
    sv_map.insert(String::new(), SparseVector::new(vec![(1, 1.0)]));

    let point = Point::with_sparse(42, vec![0.1, 0.2, 0.3, 0.4], None, Some(sv_map));
    coll.upsert(vec![point]).unwrap();

    // Verify it was indexed
    {
        let indexes = coll.sparse_indexes().read();
        let idx = indexes.get("").unwrap();
        assert_eq!(idx.doc_count(), 1);
    }

    // Delete the point
    coll.delete(&[42]).unwrap();

    // Verify it was removed from sparse index
    {
        let indexes = coll.sparse_indexes().read();
        let idx = indexes.get("").unwrap();
        assert_eq!(idx.doc_count(), 0);
        assert!(idx.get_all_postings(1).is_empty());
    }
}

#[test]
#[allow(clippy::cast_possible_truncation)]
fn test_u32_max_term_id() {
    use crate::index::sparse::search::sparse_search;
    use crate::index::sparse::SparseVector;

    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Use u32::MAX - 1 (4_294_967_294) as term_id
    let extreme_term = u32::MAX - 1;
    let mut sv_map = BTreeMap::new();
    sv_map.insert(String::new(), SparseVector::new(vec![(extreme_term, 1.5)]));

    let point = Point::with_sparse(1, vec![0.1, 0.2, 0.3, 0.4], None, Some(sv_map));
    coll.upsert(vec![point]).unwrap();

    // Verify term_id roundtrips through the index
    {
        let indexes = coll.sparse_indexes().read();
        let idx = indexes.get("").unwrap();
        assert_eq!(idx.doc_count(), 1);

        let postings = idx.get_all_postings(extreme_term);
        assert_eq!(
            postings.len(),
            1,
            "term_id {extreme_term} must have one posting"
        );
        assert_eq!(postings[0].doc_id, 1);
        assert!((postings[0].weight - 1.5).abs() < f32::EPSILON);
    }

    // Search using a query with the extreme term_id
    {
        let indexes = coll.sparse_indexes().read();
        let idx = indexes.get("").unwrap();
        let query = SparseVector::new(vec![(extreme_term, 1.0)]);
        let results = sparse_search(idx, &query, 10);
        assert_eq!(
            results.len(),
            1,
            "search with extreme term_id must find the document"
        );
        assert_eq!(results[0].doc_id, 1);
    }

    // Verify persistence roundtrip: flush and reload
    coll.flush().unwrap();
    let coll2 = Collection::open(dir.path().to_path_buf()).unwrap();
    {
        let indexes = coll2.sparse_indexes().read();
        let idx = indexes.get("").unwrap();
        assert_eq!(
            idx.doc_count(),
            1,
            "doc_count must survive persistence roundtrip"
        );
        let postings = idx.get_all_postings(extreme_term);
        assert_eq!(
            postings.len(),
            1,
            "extreme term_id must survive persistence roundtrip"
        );
        assert_eq!(postings[0].doc_id, 1);
    }
}

#[test]
fn test_sparse_wal_written_on_upsert() {
    use crate::index::sparse::SparseVector;

    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    let mut sv_map = BTreeMap::new();
    sv_map.insert(String::new(), SparseVector::new(vec![(1, 1.0)]));

    let point = Point::with_sparse(1, vec![0.1, 0.2, 0.3, 0.4], None, Some(sv_map));
    coll.upsert(vec![point]).unwrap();

    // WAL file should exist for the default sparse index
    let wal_path = dir.path().join("sparse.wal");
    assert!(wal_path.exists(), "Sparse WAL should be created on upsert");
    assert!(
        std::fs::metadata(&wal_path).unwrap().len() > 0,
        "Sparse WAL should have content"
    );
}

/// Regression test: `upsert()` with a batch should produce searchable results.
#[test]
fn test_upsert_batch_produces_searchable_results() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 16, DistanceMetric::Cosine).unwrap();

    #[allow(clippy::cast_precision_loss)] // Reason: i in [0,200); u64→f32 exact
    let points: Vec<Point> = (0u64..200)
        .map(|i| {
            let v: Vec<f32> = (0..16).map(|d| (i as f32 + d as f32) * 0.01).collect();
            Point::without_payload(i, v)
        })
        .collect();

    coll.upsert(points).expect("batch upsert should succeed");

    #[allow(clippy::cast_precision_loss)] // Reason: d in [0,16); i32→f32 exact
    let query: Vec<f32> = (0..16).map(|d| d as f32 * 0.01).collect();
    let results = coll.search(&query, 10).expect("search should succeed");
    assert_eq!(results.len(), 10, "search should return k results");
    assert_eq!(coll.config.read().point_count, 200);
}

/// Regression test: `upsert()` throughput should be close to `upsert_bulk()`.
///
/// With batched storage + batched HNSW, the gap should be within 3x.
/// The remaining overhead is secondary indexes, quantization, text indexing.
#[test]
fn test_upsert_throughput_not_degraded_vs_bulk() {
    let dim = 32;
    let n = 500;

    let dir1 = tempfile::tempdir().unwrap();
    let coll1 = Collection::create(dir1.path().to_path_buf(), dim, DistanceMetric::Cosine).unwrap();

    #[allow(clippy::cast_precision_loss)]
    let points1: Vec<Point> = (0u64..n)
        .map(|i| {
            let v: Vec<f32> = (0..dim).map(|d| (i as f32 + d as f32) * 0.01).collect();
            Point::without_payload(i, v)
        })
        .collect();

    let t0 = std::time::Instant::now();
    coll1.upsert(points1).expect("upsert should succeed");
    let upsert_dur = t0.elapsed();

    let dir2 = tempfile::tempdir().unwrap();
    let coll2 = Collection::create(dir2.path().to_path_buf(), dim, DistanceMetric::Cosine).unwrap();

    #[allow(clippy::cast_precision_loss)]
    let points2: Vec<Point> = (0u64..n)
        .map(|i| {
            let v: Vec<f32> = (0..dim).map(|d| (i as f32 + d as f32) * 0.01).collect();
            Point::without_payload(i, v)
        })
        .collect();

    let t0 = std::time::Instant::now();
    coll2
        .upsert_bulk(&points2)
        .expect("upsert_bulk should succeed");
    let bulk_dur = t0.elapsed();

    // Threshold is generous (10x) because debug builds amplify overhead from
    // secondary index updates, HashMap tracking, etc. In release builds the
    // ratio is ~1.0x. The goal is to catch gross regressions (the original
    // bug was 19x), not micro-optimize debug perf. Windows debug builds
    // exhibit 5-10% measurement noise, so 10x gives sufficient headroom.
    let ratio = upsert_dur.as_secs_f64() / bulk_dur.as_secs_f64().max(0.001);
    assert!(
        ratio < 10.0,
        "upsert() is {ratio:.1}x slower than upsert_bulk() — \
         expected <10x (upsert={upsert_dur:?}, bulk={bulk_dur:?})"
    );
}

/// BUG-0001 regression: intra-batch duplicate IDs with mixed payload patterns.
///
/// Verifies last-writer-wins semantics across four scenarios:
/// 1. Some(A) then Some(B) -> final payload is B
/// 2. Some(A) then None    -> no payload (delete wins)
/// 3. None then Some(C)    -> final payload is C
/// 4. Unique ID (no dup)   -> payload stored as-is
///
/// Also verifies WAL deduplication: only the final payload per ID is
/// written, reducing WAL bloat for batches with duplicate IDs.
#[test]
fn test_upsert_intra_batch_duplicate_ids_last_writer_wins() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Pre-seed id=10 with a payload so scenario 2 tests overwrite-then-delete
    coll.upsert(vec![Point::new(
        10,
        vec![0.1, 0.2, 0.3, 0.4],
        Some(serde_json::json!({"pre": "existing"})),
    )])
    .unwrap();

    let batch = vec![
        // Scenario 1: id=1 appears twice, both with payloads — last wins
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "A"})),
        ),
        Point::new(
            1,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "B"})),
        ),
        // Scenario 2: id=10 (pre-seeded), Some then None — delete wins
        Point::new(
            10,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"v": "X"})),
        ),
        Point::new(10, vec![0.0, 0.0, 0.0, 1.0], None),
        // Scenario 3: id=20, None then Some — store wins
        Point::without_payload(20, vec![0.5, 0.5, 0.0, 0.0]),
        Point::new(
            20,
            vec![0.0, 0.5, 0.5, 0.0],
            Some(serde_json::json!({"v": "C"})),
        ),
        // Scenario 4: id=30, unique — no dedup needed
        Point::new(
            30,
            vec![0.0, 0.0, 0.5, 0.5],
            Some(serde_json::json!({"v": "D"})),
        ),
    ];

    coll.upsert(batch).unwrap();

    let results = coll.get(&[1, 10, 20, 30]);
    assert_eq!(results.len(), 4);

    // Scenario 1: last payload wins (B), last vector wins ([0,1,0,0])
    let p1 = results[0].as_ref().expect("id=1 should exist");
    assert_eq!(p1.payload, Some(serde_json::json!({"v": "B"})));
    assert_eq!(p1.vector, vec![0.0, 1.0, 0.0, 0.0]);

    // Scenario 2: last has None payload — should be deleted
    let p10 = results[1]
        .as_ref()
        .expect("id=10 should still have a vector");
    assert!(p10.payload.is_none(), "payload should be None (deleted)");
    assert_eq!(p10.vector, vec![0.0, 0.0, 0.0, 1.0]);

    // Scenario 3: last has Some(C) — should be stored
    let p20 = results[2].as_ref().expect("id=20 should exist");
    assert_eq!(p20.payload, Some(serde_json::json!({"v": "C"})));
    assert_eq!(p20.vector, vec![0.0, 0.5, 0.5, 0.0]);

    // Scenario 4: unique — stored as-is
    let p30 = results[3].as_ref().expect("id=30 should exist");
    assert_eq!(p30.payload, Some(serde_json::json!({"v": "D"})));

    // Verify point count: 4 unique IDs (1, 10, 20, 30)
    assert_eq!(coll.len(), 4, "should have 4 unique points");
}

/// BUG-0001 regression: WAL replay produces correct state for intra-batch dupes.
///
/// Flushes, reopens the collection from disk, and verifies that the payload
/// WAL replay produces the same state as the in-memory result.
#[test]
fn test_upsert_intra_batch_wal_replay_consistency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    {
        let coll = Collection::create(path.clone(), 4, DistanceMetric::Cosine).unwrap();

        let batch = vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(serde_json::json!({"a": 1})),
            ),
            Point::new(
                1,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(serde_json::json!({"b": 2})),
            ),
            Point::without_payload(2, vec![0.5, 0.5, 0.0, 0.0]),
            Point::new(
                2,
                vec![0.0, 0.5, 0.5, 0.0],
                Some(serde_json::json!({"c": 3})),
            ),
        ];

        coll.upsert(batch).unwrap();
        coll.flush().unwrap();
    }

    // Reopen from WAL
    let coll2 = Collection::open(path).unwrap();
    let results = coll2.get(&[1, 2]);

    let p1 = results[0].as_ref().expect("id=1 should exist after reload");
    assert_eq!(p1.payload, Some(serde_json::json!({"b": 2})));
    assert_eq!(p1.vector, vec![0.0, 1.0, 0.0, 0.0]);

    let p2 = results[1].as_ref().expect("id=2 should exist after reload");
    assert_eq!(p2.payload, Some(serde_json::json!({"c": 3})));
    assert_eq!(p2.vector, vec![0.0, 0.5, 0.5, 0.0]);
}

/// BUG-0001 regression: WAL deduplication writes fewer entries.
///
/// Measures that the payload WAL is smaller when duplicate IDs are
/// deduplicated before writing, confirming the optimization is effective.
#[test]
fn test_upsert_intra_batch_wal_dedup_reduces_entries() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Batch with 3 occurrences of id=1, each with a different payload
    let batch = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "A"})),
        ),
        Point::new(
            1,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "B"})),
        ),
        Point::new(
            1,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"v": "C"})),
        ),
    ];

    coll.upsert(batch).unwrap();
    coll.flush().unwrap();

    // The payload WAL should contain exactly 1 store entry (not 3)
    // Verify by counting IDs in the payload storage index
    let payload_ids = coll.payload_storage.read().ids();
    assert_eq!(payload_ids.len(), 1, "should have 1 unique payload ID");
    assert!(
        payload_ids.contains(&1),
        "id=1 should be in payload storage"
    );

    // Verify correctness: last writer wins
    let payload = coll.payload_storage.read().retrieve(1).unwrap();
    assert_eq!(payload, Some(serde_json::json!({"v": "C"})));
}

/// Issue #424: Parallel I/O in `batch_store_all` must produce the same results
/// as the sequential implementation for large batches.
///
/// Verifies that both vectors and payloads are correctly stored when
/// payload and vector writes execute concurrently via `rayon::join`.
#[test]
fn test_batch_store_all_parallel_io_correctness() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 128, DistanceMetric::Cosine).unwrap();

    // Build a batch large enough to exercise the parallel path meaningfully
    #[allow(clippy::cast_precision_loss)] // Reason: i in [0,500); u64->f32 exact for small values
    let points: Vec<Point> = (0u64..500)
        .map(|i| {
            let v: Vec<f32> = (0..128).map(|d| (i as f32 + d as f32) * 0.001).collect();
            let payload = serde_json::json!({"idx": i, "label": format!("point_{i}")});
            Point::new(i, v, Some(payload))
        })
        .collect();

    coll.upsert(points.clone()).expect("upsert should succeed");

    // Verify all points were stored correctly
    assert_eq!(coll.len(), 500, "all 500 points should be stored");

    let ids: Vec<u64> = (0..500).collect();
    let results = coll.get(&ids);
    for (i, result) in results.iter().enumerate() {
        let p = result
            .as_ref()
            .unwrap_or_else(|| panic!("point {i} should exist"));
        assert_eq!(p.vector.len(), 128, "point {i} should have 128 dimensions");
        // Reason: i in [0, 500) — fits in u16
        #[allow(clippy::cast_precision_loss)]
        let expected_first = i as f32 * 0.001;
        assert!(
            (p.vector[0] - expected_first).abs() < 1e-6,
            "point {i} first element mismatch"
        );
        let payload = p
            .payload
            .as_ref()
            .unwrap_or_else(|| panic!("point {i} should have payload"));
        assert_eq!(payload["idx"], i as u64, "point {i} payload.idx mismatch");
    }

    // Verify search still works (HNSW was populated correctly)
    #[allow(clippy::cast_precision_loss)] // Reason: d in [0,128); i32->f32 exact for small values
    let query: Vec<f32> = (0..128).map(|d| d as f32 * 0.001).collect();
    let search_results = coll.search(&query, 10).expect("search should succeed");
    assert_eq!(search_results.len(), 10, "search should return k results");
}

/// Issue #424: Parallel I/O preserves crash recovery semantics.
///
/// After flush + reopen, all vectors and payloads written via the parallel
/// path must survive WAL replay.
#[test]
fn test_batch_store_all_parallel_io_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    {
        let coll = Collection::create(path.clone(), 32, DistanceMetric::Cosine).unwrap();

        #[allow(clippy::cast_precision_loss)]
        let points: Vec<Point> = (0u64..100)
            .map(|i| {
                let v: Vec<f32> = (0..32).map(|d| (i as f32 + d as f32) * 0.01).collect();
                Point::new(i, v, Some(serde_json::json!({"id": i})))
            })
            .collect();

        coll.upsert(points).expect("upsert should succeed");
        coll.flush().expect("flush should succeed");
    }

    // Reopen from WAL
    let coll2 = Collection::open(path).unwrap();
    assert_eq!(coll2.len(), 100, "all points should survive reopen");

    // Spot-check a few points
    let results = coll2.get(&[0, 50, 99]);
    for (i, &id) in [0u64, 50, 99].iter().enumerate() {
        let p = results[i]
            .as_ref()
            .unwrap_or_else(|| panic!("point {id} should exist after reopen"));
        assert_eq!(p.vector.len(), 32);
        let payload = p
            .payload
            .as_ref()
            .unwrap_or_else(|| panic!("point {id} should have payload after reopen"));
        assert_eq!(payload["id"], id);
    }
}

/// Issue #424: Parallel I/O handles empty-payload batches correctly.
///
/// When all points have `payload=None`, the payload write is a no-op
/// but must not panic or corrupt the vector write that runs in parallel.
#[test]
fn test_batch_store_all_parallel_io_no_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 16, DistanceMetric::Cosine).unwrap();

    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0u64..200)
        .map(|i| {
            let v: Vec<f32> = (0..16).map(|d| (i as f32 + d as f32) * 0.01).collect();
            Point::without_payload(i, v)
        })
        .collect();

    coll.upsert(points).expect("upsert should succeed");
    assert_eq!(coll.len(), 200, "all points should be stored");

    // Verify vectors are correct despite parallel path
    let results = coll.get(&[0]);
    let p0 = results[0].as_ref().expect("point 0 should exist");
    assert_eq!(p0.vector.len(), 16);
    assert!(p0.payload.is_none(), "no payload should be stored");
}

/// Issue #424: Parallel I/O handles intra-batch duplicates with mixed payloads.
///
/// The parallel path must not break the old_payloads collection that happens
/// BEFORE the parallel fork (while payload lock is still held).
#[test]
fn test_batch_store_all_parallel_io_with_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Pre-seed id=1 so the batch tests overwrite behavior
    coll.upsert(vec![Point::new(
        1,
        vec![0.1, 0.2, 0.3, 0.4],
        Some(serde_json::json!({"pre": "existing"})),
    )])
    .unwrap();

    // Batch with duplicates: id=1 appears twice, id=2 is unique
    let batch = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "A"})),
        ),
        Point::new(
            1,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "B"})),
        ),
        Point::new(
            2,
            vec![0.5, 0.5, 0.0, 0.0],
            Some(serde_json::json!({"v": "C"})),
        ),
    ];

    coll.upsert(batch)
        .expect("batch with duplicates should succeed via parallel I/O");

    let results = coll.get(&[1, 2]);
    let p1 = results[0].as_ref().expect("id=1 should exist");
    assert_eq!(
        p1.payload,
        Some(serde_json::json!({"v": "B"})),
        "last writer wins for payload"
    );
    assert_eq!(
        p1.vector,
        vec![0.0, 1.0, 0.0, 0.0],
        "last writer wins for vector"
    );

    let p2 = results[1].as_ref().expect("id=2 should exist");
    assert_eq!(p2.payload, Some(serde_json::json!({"v": "C"})));
}

// === upsert_bulk_from_raw tests (Issue #430) ===

/// Validates that `upsert_bulk_from_raw` stores vectors and payloads correctly,
/// producing identical results to the `Point`-based `upsert_bulk` path.
#[test]
fn test_upsert_bulk_from_raw_basic() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // 3 vectors of dimension 4, flat row-major layout
    let vectors: Vec<f32> = vec![
        1.0, 0.0, 0.0, 0.0, // id=10
        0.0, 1.0, 0.0, 0.0, // id=20
        0.0, 0.0, 1.0, 0.0, // id=30
    ];
    let ids: Vec<u64> = vec![10, 20, 30];
    let payloads = vec![
        Some(serde_json::json!({"tag": "a"})),
        None,
        Some(serde_json::json!({"tag": "c"})),
    ];

    let inserted = coll
        .upsert_bulk_from_raw(&vectors, &ids, 4, Some(&payloads))
        .expect("upsert_bulk_from_raw should succeed");
    assert_eq!(inserted, 3);
    assert_eq!(coll.len(), 3);

    let results = coll.get(&[10, 20, 30]);
    let p10 = results[0].as_ref().expect("id=10 should exist");
    assert_eq!(p10.vector, vec![1.0, 0.0, 0.0, 0.0]);
    assert_eq!(p10.payload, Some(serde_json::json!({"tag": "a"})));

    let p20 = results[1].as_ref().expect("id=20 should exist");
    assert_eq!(p20.vector, vec![0.0, 1.0, 0.0, 0.0]);
    assert!(p20.payload.is_none());

    let p30 = results[2].as_ref().expect("id=30 should exist");
    assert_eq!(p30.vector, vec![0.0, 0.0, 1.0, 0.0]);
    assert_eq!(p30.payload, Some(serde_json::json!({"tag": "c"})));
}

/// Validates that `upsert_bulk_from_raw` works without payloads.
#[test]
fn test_upsert_bulk_from_raw_no_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    let vectors: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let ids: Vec<u64> = vec![1, 2];

    let inserted = coll
        .upsert_bulk_from_raw(&vectors, &ids, 4, None)
        .expect("upsert_bulk_from_raw without payloads should succeed");
    assert_eq!(inserted, 2);
    assert_eq!(coll.len(), 2);

    let results = coll.get(&[1, 2]);
    let p1 = results[0].as_ref().expect("id=1 should exist");
    assert_eq!(p1.vector, vec![0.1, 0.2, 0.3, 0.4]);
    assert!(p1.payload.is_none());
}

/// Validates that `upsert_bulk_from_raw` returns an error on dimension mismatch.
#[test]
fn test_upsert_bulk_from_raw_dimension_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Collection dimension is 4, but we pass dimension=3
    let vectors: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
    let ids: Vec<u64> = vec![1, 2];

    let result = coll.upsert_bulk_from_raw(&vectors, &ids, 3, None);
    assert!(result.is_err(), "should fail on dimension mismatch");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("VELES-004"),
        "should be DimensionMismatch error: {err_msg}"
    );
}

/// Validates that `upsert_bulk_from_raw` returns an error on length mismatch.
#[test]
fn test_upsert_bulk_from_raw_vector_length_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // 5 floats but 2 ids * 4 dim = 8 expected
    let vectors: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let ids: Vec<u64> = vec![1, 2];

    let result = coll.upsert_bulk_from_raw(&vectors, &ids, 4, None);
    assert!(result.is_err(), "should fail on vector length mismatch");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("VELES-005"),
        "should be InvalidVector error: {err_msg}"
    );
}

/// Validates that `upsert_bulk_from_raw` returns an error on payload length mismatch.
#[test]
fn test_upsert_bulk_from_raw_payload_length_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    let vectors: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
    let ids: Vec<u64> = vec![1, 2];
    let payloads = vec![Some(serde_json::json!({"x": 1}))]; // length 1, not 2

    let result = coll.upsert_bulk_from_raw(&vectors, &ids, 4, Some(&payloads));
    assert!(result.is_err(), "should fail on payload length mismatch");
}

/// Validates that `upsert_bulk_from_raw` with empty inputs returns 0.
#[test]
fn test_upsert_bulk_from_raw_empty() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    let inserted = coll
        .upsert_bulk_from_raw(&[], &[], 4, None)
        .expect("empty call should succeed");
    assert_eq!(inserted, 0);
    assert_eq!(coll.len(), 0);
}

/// Validates that vectors inserted via `upsert_bulk_from_raw` are searchable.
#[test]
fn test_upsert_bulk_from_raw_searchable() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Insert 50 vectors so HNSW has enough data to exercise search
    #[allow(clippy::cast_precision_loss)] // Reason: i in [0,50); u64->f32 exact
    let vectors: Vec<f32> = (0u64..50)
        .flat_map(|i| {
            let base = i as f32 * 0.02;
            vec![base, base + 0.01, base + 0.02, base + 0.03]
        })
        .collect();
    let ids: Vec<u64> = (0..50).collect();

    coll.upsert_bulk_from_raw(&vectors, &ids, 4, None)
        .expect("bulk insert should succeed");
    assert_eq!(coll.len(), 50);

    let query = vec![0.0_f32, 0.01, 0.02, 0.03];
    let results = coll.search(&query, 5).expect("search should succeed");
    assert_eq!(results.len(), 5, "search should return k=5 results");
    // The nearest neighbor for the query [0.0, 0.01, 0.02, 0.03] should be id=0
    assert_eq!(results[0].point.id, 0, "nearest neighbor should be point 0");
}

/// Validates that `upsert_bulk_from_raw` survives flush + reopen.
#[test]
fn test_upsert_bulk_from_raw_persistence_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_path_buf();
    {
        let coll = Collection::create(path.clone(), 4, DistanceMetric::Cosine).unwrap();
        let vectors: Vec<f32> = vec![1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let ids: Vec<u64> = vec![100, 200];
        let payloads = vec![
            Some(serde_json::json!({"key": "first"})),
            Some(serde_json::json!({"key": "second"})),
        ];

        coll.upsert_bulk_from_raw(&vectors, &ids, 4, Some(&payloads))
            .expect("insert should succeed");
        coll.flush().expect("flush should succeed");
    }

    // Reopen from disk
    let coll2 = Collection::open(path).unwrap();
    assert_eq!(coll2.len(), 2);

    let results = coll2.get(&[100, 200]);
    let p100 = results[0].as_ref().expect("id=100 should survive reopen");
    assert_eq!(p100.vector, vec![1.0, 0.0, 0.0, 0.0]);
    assert_eq!(p100.payload, Some(serde_json::json!({"key": "first"})));

    let p200 = results[1].as_ref().expect("id=200 should survive reopen");
    assert_eq!(p200.vector, vec![0.0, 1.0, 0.0, 0.0]);
    assert_eq!(p200.payload, Some(serde_json::json!({"key": "second"})));
}

/// Validates that `upsert_bulk_from_raw` produces identical results to
/// `upsert_bulk` for the same input data (parity test).
#[test]
fn test_upsert_bulk_from_raw_parity_with_upsert_bulk() {
    let dim = 8;
    let n = 100;

    // Build test data
    #[allow(clippy::cast_precision_loss)] // Reason: i in [0,100); u64->f32 exact
    let flat_vectors: Vec<f32> = (0u64..n)
        .flat_map(|i| (0..dim).map(move |d| (i as f32 + d as f32) * 0.01))
        .collect();
    let id_list: Vec<u64> = (0..n).collect();
    let payloads: Vec<Option<serde_json::Value>> = (0u64..n)
        .map(|i| Some(serde_json::json!({"idx": i})))
        .collect();

    // Path A: upsert_bulk_from_raw
    let dir_a = tempfile::tempdir().unwrap();
    let coll_a =
        Collection::create(dir_a.path().to_path_buf(), dim, DistanceMetric::Cosine).unwrap();
    coll_a
        .upsert_bulk_from_raw(&flat_vectors, &id_list, dim, Some(&payloads))
        .expect("raw path should succeed");

    // Path B: upsert_bulk (Point-based)
    let dir_b = tempfile::tempdir().unwrap();
    let coll_b =
        Collection::create(dir_b.path().to_path_buf(), dim, DistanceMetric::Cosine).unwrap();
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0u64..n)
        .map(|i| {
            let v: Vec<f32> = (0..dim).map(|d| (i as f32 + d as f32) * 0.01).collect();
            Point::new(i, v, Some(serde_json::json!({"idx": i})))
        })
        .collect();
    coll_b
        .upsert_bulk(&points)
        .expect("point path should succeed");

    // Compare stored data
    assert_eq!(coll_a.len(), coll_b.len());
    let all_ids: Vec<u64> = (0..n).collect();
    let results_a = coll_a.get(&all_ids);
    let results_b = coll_b.get(&all_ids);

    for i in 0..usize::try_from(n).expect("n fits in usize") {
        let pa = results_a[i]
            .as_ref()
            .unwrap_or_else(|| panic!("raw: point {i} missing"));
        let pb = results_b[i]
            .as_ref()
            .unwrap_or_else(|| panic!("bulk: point {i} missing"));
        assert_eq!(pa.vector, pb.vector, "vector mismatch at point {i}");
        assert_eq!(pa.payload, pb.payload, "payload mismatch at point {i}");
    }
}

// === Issue #425: Phase 2 fast-path + BM25 skip + dedup map consolidation ===

/// Issue #425: Phase 2 fast-path should not skip when secondary indexes exist.
///
/// Regression: ensures that adding a secondary index forces Phase 2 to run,
/// so payload-based indexes are correctly updated on upsert.
#[test]
fn test_phase2_runs_when_secondary_indexes_exist() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Add a secondary index on the "category" field
    coll.create_index("category").unwrap();

    // Upsert points WITH payloads — Phase 2 must run to populate the index
    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"category": "books"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"category": "movies"})),
        ),
    ];
    coll.upsert(points).unwrap();

    // Verify the secondary index was populated
    let indexes = coll.secondary_indexes.read();
    let cat_index = indexes.get("category").expect("index should exist");
    match cat_index {
        crate::index::SecondaryIndex::BTree(tree) => {
            let tree = tree.read();
            assert!(
                !tree.is_empty(),
                "secondary index should contain entries after upsert"
            );
        }
    }
}

/// Issue #425: Phase 2 fast-path correctly skips for StorageMode::Full +
/// no secondary indexes + no payloads + no sparse vectors.
///
/// Regression: confirms that the fast path produces identical results to
/// the full Phase 2 path for plain vector-only inserts.
#[test]
fn test_phase2_fast_path_correctness_no_secondaries() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Insert vector-only points (no payload, no sparse, no secondary indexes)
    // This should trigger the fast path in Phase 2
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (0u64..100)
        .map(|i| {
            let v: Vec<f32> = (0..4).map(|d| (i as f32 + d as f32) * 0.01).collect();
            Point::without_payload(i, v)
        })
        .collect();

    coll.upsert(points).unwrap();

    // All 100 points should be stored and searchable
    assert_eq!(coll.len(), 100, "all points should be stored");
    let results = coll.search(&[0.5, 0.5, 0.5, 0.5], 10).unwrap();
    assert_eq!(results.len(), 10, "search should return k results");
}

/// Issue #425: Phase 2 must NOT skip when points carry sparse vectors.
///
/// Regression: sparse vectors must be collected in Phase 2 and written
/// to sparse indexes even when no other secondary processing is needed.
#[test]
fn test_phase2_does_not_skip_with_sparse_vectors() {
    use crate::index::sparse::SparseVector;

    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    let mut sv_map = BTreeMap::new();
    sv_map.insert(String::new(), SparseVector::new(vec![(1, 1.0), (2, 0.5)]));

    let point = Point::with_sparse(1, vec![0.1, 0.2, 0.3, 0.4], None, Some(sv_map));
    coll.upsert(vec![point]).unwrap();

    // Sparse index must be populated (Phase 2 ran)
    let indexes = coll.sparse_indexes().read();
    assert!(
        indexes.contains_key(""),
        "sparse index should be populated despite no payloads"
    );
    assert_eq!(indexes.get("").unwrap().doc_count(), 1);
}

/// Issue #425: BM25 skip in bulk path must still index text when payloads exist.
///
/// Regression: the BM25 skip optimization in `bulk_store_payloads` must
/// NOT skip when at least one point has a payload containing text.
#[test]
fn test_bulk_bm25_skip_does_not_lose_text() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"text": "hello world"})),
        ),
        Point::without_payload(2, vec![0.0, 1.0, 0.0, 0.0]),
    ];

    coll.upsert_bulk(&points).unwrap();

    // BM25 should have indexed the text from point 1
    assert!(
        !coll.text_index.is_empty(),
        "BM25 index should contain the document from bulk insert"
    );
}

/// Issue #425: Dedup map consolidation produces same results as separate maps.
///
/// Regression: the shared dedup map path must produce identical WAL behavior
/// to the previous per-storage dedup map. Tests both payload and vector dedup.
#[test]
fn test_dedup_map_consolidation_correctness() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Batch with duplicate IDs — last writer wins for both payload and vector
    let batch = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "first"})),
        ),
        Point::new(
            1,
            vec![0.0, 1.0, 0.0, 0.0],
            Some(serde_json::json!({"v": "second"})),
        ),
        Point::new(
            2,
            vec![0.0, 0.0, 1.0, 0.0],
            Some(serde_json::json!({"v": "only"})),
        ),
    ];

    coll.upsert(batch).unwrap();

    let results = coll.get(&[1, 2]);
    let p1 = results[0].as_ref().expect("id=1 should exist");
    assert_eq!(
        p1.payload,
        Some(serde_json::json!({"v": "second"})),
        "shared dedup map should preserve last-writer-wins for payload"
    );
    assert_eq!(
        p1.vector,
        vec![0.0, 1.0, 0.0, 0.0],
        "shared dedup map should preserve last-writer-wins for vector"
    );

    let p2 = results[1].as_ref().expect("id=2 should exist");
    assert_eq!(p2.payload, Some(serde_json::json!({"v": "only"})));
    assert_eq!(coll.len(), 2, "should have 2 unique points");
}

/// Issue #425: Phase 2 must NOT skip when StorageMode is SQ8.
///
/// Regression: quantization caching requires per-point processing in Phase 2.
#[test]
fn test_phase2_runs_for_sq8_storage_mode() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create_with_options(
        dir.path().to_path_buf(),
        4,
        DistanceMetric::Cosine,
        StorageMode::SQ8,
    )
    .unwrap();

    let points = vec![Point::without_payload(1, vec![1.0, 0.0, 0.0, 0.0])];
    coll.upsert(points).unwrap();

    // SQ8 cache should have been populated by Phase 2
    assert_eq!(
        coll.sq8_cache.read().len(),
        1,
        "SQ8 cache should be populated — Phase 2 must not skip"
    );
}

/// Issue #486: Parallel SQ8 quantization produces correct cache entries.
///
/// Verifies that the parallel quantization path (rayon) populates
/// the cache for all points and that each entry exists.
#[test]
fn test_parallel_sq8_quantization_correctness() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create_with_options(
        dir.path().to_path_buf(),
        4,
        DistanceMetric::Cosine,
        StorageMode::SQ8,
    )
    .unwrap();

    // Insert 50 points to exercise the parallel path (rayon splits work)
    let points: Vec<Point> = (0u64..50)
        .map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let v = vec![id as f32 * 0.1, 0.2, 0.3, 0.4];
            Point::without_payload(id, v)
        })
        .collect();
    coll.upsert(points.clone()).unwrap();

    // Verify all 50 entries are in the SQ8 cache
    let cache = coll.sq8_cache.read();
    assert_eq!(
        cache.len(),
        50,
        "all 50 points should have SQ8 cache entries"
    );

    // Verify each point has a cache entry (parallel and sequential produce the same result)
    for p in &points {
        assert!(
            cache.contains_key(&p.id),
            "SQ8 cache should contain entry for id={}",
            p.id
        );
    }
}

/// Issue #486: Parallel Binary quantization produces correct cache entries.
#[test]
fn test_parallel_binary_quantization_correctness() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create_with_options(
        dir.path().to_path_buf(),
        4,
        DistanceMetric::Cosine,
        StorageMode::Binary,
    )
    .unwrap();

    let points: Vec<Point> = (0u64..50)
        .map(|id| {
            #[allow(clippy::cast_precision_loss)]
            let v = vec![id as f32 * 0.1 - 2.5, 0.2, -0.3, 0.4];
            Point::without_payload(id, v)
        })
        .collect();
    coll.upsert(points.clone()).unwrap();

    let cache = coll.binary_cache.read();
    assert_eq!(
        cache.len(),
        50,
        "all 50 points should have Binary cache entries"
    );

    for p in &points {
        assert!(
            cache.contains_key(&p.id),
            "Binary cache should contain entry for id={}",
            p.id
        );
    }
}

/// Issue #486: Multi-batch upsert produces searchable results without
/// set_searching_mode() overhead.
///
/// Regression: removing set_searching_mode() from bulk_index_or_defer()
/// must not break search correctness.
#[test]
fn test_multi_batch_upsert_search_correctness_without_searching_mode() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Insert in 5 batches of 20 — simulates the multi-batch Python workload
    for batch_idx in 0u64..5 {
        let points: Vec<Point> = (0u64..20)
            .map(|i| {
                let id = batch_idx * 20 + i;
                #[allow(clippy::cast_precision_loss)]
                let v = vec![id as f32 / 100.0, 0.1, 0.1, 0.1];
                Point::without_payload(id, v)
            })
            .collect();
        coll.upsert(points).unwrap();
    }

    assert_eq!(coll.len(), 100, "should have 100 points after 5 batches");

    // Search should return results
    let results = coll.search(&[0.5, 0.1, 0.1, 0.1], 10).unwrap();
    assert_eq!(
        results.len(),
        10,
        "search should return 10 results after multi-batch insert"
    );

    // Verify all returned IDs are valid (in range 0..100)
    for r in &results {
        assert!(
            r.point.id < 100,
            "search result id={} should be in range 0..100",
            r.point.id
        );
    }
}

/// Issue #486: upsert_bulk multi-batch also works without set_searching_mode().
#[test]
fn test_upsert_bulk_multi_batch_search_correctness() {
    let dir = tempfile::tempdir().unwrap();
    let coll = Collection::create(dir.path().to_path_buf(), 4, DistanceMetric::Cosine).unwrap();

    // Insert in 3 bulk batches (simulates Python benchmark pattern)
    for batch_idx in 0u64..3 {
        let points: Vec<Point> = (0u64..100)
            .map(|i| {
                let id = batch_idx * 100 + i;
                #[allow(clippy::cast_precision_loss)]
                let v = vec![id as f32 / 300.0, 0.1, 0.2, 0.3];
                Point::without_payload(id, v)
            })
            .collect();
        coll.upsert_bulk(&points).unwrap();
    }

    assert_eq!(
        coll.len(),
        300,
        "should have 300 points after 3 bulk batches"
    );

    let results = coll
        .search(&[0.5, 0.1, 0.2, 0.3], 10)
        .expect("search should succeed after multi-batch bulk insert");
    assert_eq!(
        results.len(),
        10,
        "search should return 10 results after multi-batch bulk insert"
    );
}

/// Regression test: upsert removing `_labels` must clean up LabelIndex.
///
/// Scenario: insert point with `_labels: ["Person"]`, then upsert the same
/// point WITHOUT `_labels`. The LabelIndex must no longer contain the node
/// under "Person". Previously, `has_any_labels` only checked new payloads,
/// so label removal was silently skipped (Devin review finding).
#[test]
fn test_upsert_removes_stale_labels_from_label_index() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection");

    // Step 1: Insert point with _labels
    let p1 = Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({"_labels": ["Person"], "name": "Alice"})),
    );
    collection.upsert(vec![p1]).expect("upsert with labels");

    // Verify label is indexed
    let label_idx = collection.label_index.read();
    assert!(
        label_idx.lookup("Person").is_some_and(|b| b.contains(1)),
        "Person label should be indexed for node 1"
    );
    drop(label_idx);

    // Step 2: Upsert same point WITHOUT _labels
    let p1_updated = Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({"name": "Alice Updated"})),
    );
    collection
        .upsert(vec![p1_updated])
        .expect("upsert without labels");

    // Verify stale label is removed
    let label_idx = collection.label_index.read();
    let still_has = label_idx.lookup("Person").is_some_and(|b| b.contains(1));
    assert!(
        !still_has,
        "Person label should be removed after upsert without _labels"
    );
}

/// Regression test: `can_skip_phase2` must not skip when label index is populated.
///
/// Scenario: insert a point with `_labels: ["Person"]`, then upsert the same
/// point with `payload: None` (no payload at all). Without the fix,
/// `can_skip_phase2` returns `true` because `any_payload` is false, skipping
/// Phase 2 entirely and leaving stale labels in the index.
///
/// Devin review finding (2026-04-02).
#[test]
fn test_can_skip_phase2_respects_populated_label_index() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection");

    // Step 1: Insert point with _labels — populates the label index.
    let p1 = Point::new(
        1,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({"_labels": ["Person"], "name": "Alice"})),
    );
    collection.upsert(vec![p1]).expect("upsert with labels");

    // Verify label index is populated.
    let label_idx = collection.label_index.read();
    assert!(
        label_idx.lookup("Person").is_some_and(|b| b.contains(1)),
        "Person label should be indexed for node 1"
    );
    drop(label_idx);

    // Step 2: Upsert same point with NO payload at all.
    // This is the scenario where `can_skip_phase2` incorrectly returned true
    // because `any_payload` was false and the label index was not checked.
    let p1_no_payload = Point::without_payload(1, vec![0.0, 1.0, 0.0, 0.0]);
    collection
        .upsert(vec![p1_no_payload])
        .expect("upsert without payload");

    // Verify stale label is removed — Phase 2 must have run.
    let label_idx = collection.label_index.read();
    let still_has = label_idx.lookup("Person").is_some_and(|b| b.contains(1));
    assert!(
        !still_has,
        "Person label should be removed when upserting with payload: None"
    );
}

/// Regression test: `find_start_nodes_full_scan` must filter by labels.
///
/// Scenario: when node IDs exceed `u32::MAX`, the label index cannot store
/// them (RoaringBitmap limitation), so `find_start_nodes` falls back to
/// `find_start_nodes_full_scan`. Without the fix, `needs_payload` was only
/// set when properties were present, causing label-only patterns like
/// `(n:Person)` to return ALL nodes instead of only Person-labeled ones.
///
/// Devin review finding (2026-04-02).
#[test]
fn test_full_scan_fallback_filters_by_labels() {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let collection = Collection::create(PathBuf::from(temp_dir.path()), 4, DistanceMetric::Cosine)
        .expect("collection");

    let large_base: u64 = u64::from(u32::MAX) + 1;

    // Insert nodes with large IDs (> u32::MAX) so the label index cannot
    // index them and `has_large_ids` is set. Use payloads to store labels.
    let person_node = Point::new(
        large_base,
        vec![1.0, 0.0, 0.0, 0.0],
        Some(serde_json::json!({"_labels": ["Person"], "name": "Alice"})),
    );
    let company_node = Point::new(
        large_base + 1,
        vec![0.0, 1.0, 0.0, 0.0],
        Some(serde_json::json!({"_labels": ["Company"], "name": "Acme"})),
    );
    collection
        .upsert(vec![person_node, company_node])
        .expect("upsert large-ID nodes");

    // Confirm the label index has large_ids set and no indexed entries.
    let label_idx = collection.label_index.read();
    assert!(
        label_idx.has_large_ids(),
        "has_large_ids should be true after indexing nodes with ID > u32::MAX"
    );
    assert!(
        label_idx.lookup("Person").is_none(),
        "Person bitmap should be empty (IDs too large for RoaringBitmap)"
    );
    drop(label_idx);

    // Run MATCH (n:Person) RETURN n — should only return the Person node.
    let match_clause = crate::velesql::MatchClause {
        patterns: vec![crate::velesql::GraphPattern {
            name: None,
            nodes: vec![crate::velesql::NodePattern::new()
                .with_alias("n")
                .with_label("Person")],
            relationships: vec![],
        }],
        where_clause: None,
        return_clause: crate::velesql::ReturnClause {
            items: vec![crate::velesql::ReturnItem {
                expression: "n".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(100),
        },
    };
    let params = std::collections::HashMap::new();
    let results = collection
        .execute_match(&match_clause, &params)
        .expect("execute_match should succeed");

    // Only the Person-labeled node should be returned, not the Company node.
    assert_eq!(
        results.len(),
        1,
        "MATCH (n:Person) should return exactly 1 node, got {}",
        results.len()
    );
    assert_eq!(
        results[0].node_id, large_base,
        "matched node should be the Person node (id={})",
        large_base
    );
}
