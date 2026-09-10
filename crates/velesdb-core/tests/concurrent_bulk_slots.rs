//! Concurrent `upsert_bulk` and `upsert` must never give one arena slot to
//! two ids (#2246).
//!
//! Before #2246 the bulk path's direct writer wrote each vector at a slot the
//! mapping counter predicted, while a graph insert appended at the arena's
//! length and then remapped its ids to the slots it actually got; when a graph
//! insert landed between a direct write's registration and its write, both
//! ids pointed at one slot and the later write overwrote the earlier vector.
//! Checked through the exhaustive search, which reads the arena: every id must
//! find its own vector — once with the builder draining as it goes, and once
//! with it holding every bulk id, so the direct writer's own slots are the
//! ones read.

#![cfg(feature = "persistence")]

use std::sync::Arc;
use tempfile::TempDir;
use velesdb_core::collection::streaming::AsyncIndexBuilderConfig;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::{Point, SearchQuality, VectorCollection};

const DIM: usize = 8;
const ROUNDS: u64 = 400;
const BATCH: u64 = 8;
const BULK_BASE: u64 = 1_000_000;

/// Dispersed values (xorshift), so every point is its own nearest neighbour.
fn vector(id: u64) -> Vec<f32> {
    let mut state = id.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            #[allow(
                clippy::cast_precision_loss,
                reason = "the top 24 bits of the state are exact in an f32"
            )]
            let unit = (state >> 40) as f32 / (1u64 << 24) as f32;
            unit
        })
        .collect()
}

fn batch(base: u64, round: u64) -> Vec<Point> {
    (0..BATCH)
        .map(|i| {
            let id = base + round * BATCH + i;
            Point::without_payload(id, vector(id))
        })
        .collect()
}

/// Runs the two writers against a collection whose builder merges every
/// `merge_threshold` pending points.
fn race(dir: &TempDir, merge_threshold: usize) -> Arc<VectorCollection> {
    let config = AsyncIndexBuilderConfig {
        merge_threshold,
        segment_count: Some(2),
    };
    let collection = Arc::new(
        VectorCollection::create_with_async_builder(
            dir.path().join("slots"),
            DIM,
            DistanceMetric::Euclidean,
            config,
        )
        .expect("test: create with async builder"),
    );

    let bulk = {
        let collection = Arc::clone(&collection);
        std::thread::spawn(move || {
            for round in 0..ROUNDS {
                collection
                    .upsert_bulk(&batch(BULK_BASE, round))
                    .expect("test: upsert_bulk");
            }
        })
    };
    let plain = {
        let collection = Arc::clone(&collection);
        std::thread::spawn(move || {
            for round in 0..ROUNDS {
                collection.upsert(batch(0, round)).expect("test: upsert");
            }
        })
    };
    bulk.join().expect("test: bulk writer");
    plain.join().expect("test: plain writer");
    collection
}

/// The ids whose exhaustive self-query does not return them first.
fn lost(collection: &VectorCollection) -> Vec<u64> {
    (0..ROUNDS * BATCH)
        .chain(BULK_BASE..BULK_BASE + ROUNDS * BATCH)
        .filter(|&id| {
            let hits = collection
                .search_with_quality(&vector(id), 1, SearchQuality::Perfect)
                .expect("test: exhaustive search");
            hits.first().map(|hit| hit.point.id) != Some(id)
        })
        .collect()
}

#[test]
fn concurrent_bulk_and_plain_upserts_keep_every_vector_in_its_own_slot() {
    let dir = TempDir::new().expect("test: tempdir");
    let collection = race(&dir, 64);
    collection.flush().expect("test: flush drains the builder");
    let lost = lost(&collection);
    assert!(
        lost.is_empty(),
        "{} ids no longer find their own vector, e.g. {:?}",
        lost.len(),
        &lost[..lost.len().min(8)]
    );
}

/// With the builder holding every bulk id, each one is still mapped to the
/// slot the direct writer gave it: those slots are the ones read here.
#[test]
fn direct_written_slots_hold_their_own_vectors_before_the_builder_drains() {
    let dir = TempDir::new().expect("test: tempdir");
    let bulk_total = usize::try_from(ROUNDS * BATCH).expect("test: fits a usize");
    let collection = race(&dir, bulk_total + 1);
    let lost = lost(&collection);
    assert!(
        lost.is_empty(),
        "{} ids no longer find their own vector before the drain, e.g. {:?}",
        lost.len(),
        &lost[..lost.len().min(8)]
    );
}
