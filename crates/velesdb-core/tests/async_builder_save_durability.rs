//! A save that runs while the async index builder still holds vectors must
//! leave a store whose graph has them (#2246).
//!
//! `upsert_bulk`'s V2 path registers each id's mapping and writes its vector at
//! once, and leaves the graph insert to the `AsyncIndexBuilder`. A save in that
//! window persists mappings for ids the graph has never seen. Recovery
//! re-indexes only ids with NO mapping, so after a crash those points would be
//! stored, mapped — and unreachable by graph search. `flush` drains the builder
//! before it saves; these tests hold the other two saves to the same rule.
//!
//! A crash is simulated by copying the directory right after the save and
//! opening the copy: what a process that died there leaves on disk.

#![cfg(feature = "persistence")]

use std::path::{Path, PathBuf};
use tempfile::TempDir;
use velesdb_core::collection::streaming::AsyncIndexBuilderConfig;
use velesdb_core::distance::DistanceMetric;
use velesdb_core::{Point, VectorCollection};

const DIM: usize = 8;
/// Above the 100-point brute-force shortcut, so a search walks the graph.
const POINTS: u64 = 300;
/// Well above `POINTS`, so the builder holds every insert until drained.
const MERGE_THRESHOLD: usize = 10_000;
const PROBES: [u64; 3] = [0, POINTS / 2, POINTS - 1];

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

/// A collection whose bulk inserts are all still pending in the builder.
fn pending_collection(dir: PathBuf) -> VectorCollection {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: MERGE_THRESHOLD,
        segment_count: Some(2),
    };
    let collection =
        VectorCollection::create_with_async_builder(dir, DIM, DistanceMetric::Euclidean, config)
            .expect("test: create with async builder");
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::without_payload(id, vector(id)))
        .collect();
    assert_eq!(
        collection.upsert_bulk(&points).expect("test: upsert_bulk"),
        points.len()
    );
    collection
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("test: create snapshot dir");
    for entry in std::fs::read_dir(from).expect("test: read dir") {
        let entry = entry.expect("test: dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("test: file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("test: copy file");
        }
    }
}

/// Opens what a crash right now would leave, and returns the probes graph
/// search cannot find there.
fn missing_after_crash(live: &Path, tmp: &TempDir, label: &str) -> Vec<u64> {
    let snapshot = tmp.path().join(label);
    copy_dir(live, &snapshot);
    let reopened = VectorCollection::open(snapshot).expect("test: open the crash snapshot");
    PROBES
        .into_iter()
        .filter(|&id| {
            let hits = reopened.search(&vector(id), 1).expect("test: search");
            hits.first().map(|hit| hit.point.id) != Some(id)
        })
        .collect()
}

/// CONTROL: a crash with no save at all loses nothing — recovery re-indexes
/// every stored id that has no mapping. Without it, the tests below could fail
/// for a reason that has nothing to do with the save they are about.
#[test]
fn a_crash_before_any_save_loses_nothing() {
    let tmp = TempDir::new().expect("test: tempdir");
    let live = tmp.path().join("live");
    let _collection = pending_collection(live.clone());
    assert_eq!(
        missing_after_crash(&live, &tmp, "no-save"),
        Vec::<u64>::new()
    );
}

#[test]
fn compacting_with_inserts_pending_saves_a_graph_that_has_them() {
    let tmp = TempDir::new().expect("test: tempdir");
    let live = tmp.path().join("live");
    let collection = pending_collection(live.clone());
    collection.compact_storage().expect("test: compact");
    assert_eq!(
        missing_after_crash(&live, &tmp, "compacted"),
        Vec::<u64>::new(),
        "points graph search cannot reach after a crash that followed compaction"
    );
}

#[test]
fn reordering_with_inserts_pending_saves_a_graph_that_has_them() {
    let tmp = TempDir::new().expect("test: tempdir");
    let live = tmp.path().join("live");
    let collection = pending_collection(live.clone());
    collection.reorder_for_locality().expect("test: reorder");
    assert_eq!(
        missing_after_crash(&live, &tmp, "reordered"),
        Vec::<u64>::new(),
        "points graph search cannot reach after a crash that followed a reorder"
    );
}
