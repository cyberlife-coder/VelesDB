#![cfg(all(test, feature = "persistence"))]
//! Tests for open-time quantizer restore.
//!
//! `train_opq` refuses metrics an orthogonal transform does not preserve, but
//! that guard only covers codebooks trained from now on. A `codebook.pq`
//! written before it exists — or copied in from another collection — reaches
//! the process through `restore_persisted_pq` instead, so the same rule has to
//! hold there.

#![allow(clippy::cast_precision_loss)]

use std::path::PathBuf;

use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;

const DIM: usize = 8;

fn training_vectors() -> Vec<Vec<f32>> {
    (0..64_u64)
        .map(|i| {
            (0..DIM)
                .map(|d| ((i as f32) * 0.37 + (d as f32) * 1.13).sin())
                .collect()
        })
        .collect()
}

fn points(vectors: &[Vec<f32>]) -> Vec<Point> {
    vectors
        .iter()
        .enumerate()
        .map(|(i, v)| Point::without_payload(i as u64, v.clone()))
        .collect()
}

/// Writes an OPQ codebook (rotation included) into `dir`, as a collection
/// trained before the metric guard existed would have left behind.
fn plant_opq_codebook(dir: &std::path::Path, vectors: &[Vec<f32>]) {
    let pq = crate::quantization::train_opq(vectors, 2, 4, true, 5).expect("test: train OPQ");
    assert!(
        pq.rotation.is_some(),
        "test setup: the planted codebook must carry a rotation"
    );
    pq.save_codebook(dir).expect("test: save codebook");
}

fn restore_with_metric(metric: DistanceMetric) -> bool {
    let temp = tempfile::tempdir().expect("test: temp dir");
    let vectors = training_vectors();

    {
        let coll = Collection::create(PathBuf::from(temp.path()), DIM, metric)
            .expect("test: create collection");
        coll.upsert(points(&vectors)).expect("test: upsert");
        // Restore is gated on the storage mode, which training flips alongside
        // writing the codebook; without it `restore_persisted_pq` never runs
        // and every assertion below would pass for the wrong reason.
        {
            let mut config = coll.config_write();
            config.storage_mode = crate::StorageMode::ProductQuantization;
        }
        coll.save_config().expect("test: persist config");
        coll.flush_full().expect("test: flush");
    }

    plant_opq_codebook(temp.path(), &vectors);

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("test: reopen");
    let installed = reopened.pq_quantizer_read().is_some();
    installed
}

/// A rotated codebook must not be installed on a metric the rotation destroys.
///
/// Dropping just the rotation would not repair it: the codes themselves live
/// in the rotated basis. Degrading the whole quantizer costs speed and keeps
/// the answers right, which is what the dimension-mismatch arm beside it does.
#[test]
fn a_rotated_codebook_is_refused_on_a_rotation_sensitive_metric() {
    for metric in [DistanceMetric::Hamming, DistanceMetric::Jaccard] {
        assert!(
            !restore_with_metric(metric),
            "{metric:?}: an OPQ codebook must not be installed"
        );
    }
}

/// The same codebook still restores on the metrics a rotation preserves.
#[test]
fn a_rotated_codebook_still_restores_on_rotation_invariant_metrics() {
    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::DotProduct,
    ] {
        assert!(
            restore_with_metric(metric),
            "{metric:?}: an OPQ codebook must still restore"
        );
    }
}
