#![cfg(all(test, feature = "persistence"))]

use crate::{distance::DistanceMetric, point::Point, quantization::StorageMode, Collection};
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
