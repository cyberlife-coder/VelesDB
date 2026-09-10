#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)] // indices into f32 coordinates, deliberately

//! `reorder_for_locality` must leave the two files it touches consistent.
//!
//! Since `.vectors` became the graph's arena, the permutation lands in the
//! **durable** store the moment it runs, while the adjacency it must match
//! lives in `.graph`. Leaving the save to a later `flush_full` opened a window
//! where a crash left every node id resolving to the wrong vector — silently,
//! because both files still parse. Measured before the fix:
//!
//! ```text
//! reorder_for_locality()  ->  Ok
//! .vectors  ->  modified without save
//! .graph    ->  unchanged (old numbering)
//! ```
//!
//! Before the arena adoption the permutation touched a disposable
//! `hnsw-{token}.arena`, so no window existed. This file pins the invariant the
//! adoption made necessary.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use velesdb_core::{Database, DistanceMetric, Point};

const DIM: usize = 16;
/// Above `REORDER_THRESHOLD` (1 000), or the reorder is a documented no-op.
const POINTS: u64 = 3_000;

const _: () = assert!(
    POINTS > 1_000,
    "below the reorder threshold the permutation never runs and this file proves nothing"
);

/// Deterministic but dispersed.
///
/// The first draft used `(id * 31 + j) % 997`, which repeats past 997 ids: on a
/// 3 000-point fixture two thirds of the vectors were duplicates and the
/// nearest-neighbour control failed, returning 2231 for a query built from
/// 1234. The control caught the fixture before the assertion it guards could
/// pass on a coincidence.
fn vector(id: u64) -> Vec<f32> {
    let mut x = id.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 10_000) as f32 / 10_000.0
        })
        .collect()
}

fn find(root: &Path, ext: &str) -> PathBuf {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("test: read_dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == ext) {
                return path;
            }
        }
    }
    panic!("test: no .{ext} under {}", root.display());
}

fn digest(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    std::fs::read(path).expect("test: read").hash(&mut hasher);
    hasher.finish()
}

/// The permutation and the adjacency reach disk together.
///
/// Asserted on the FILES rather than through a reopen: a graceful drop flushes,
/// which would repair the inconsistency before any assertion could see it. The
/// window this guards is the one a crash falls into, and only the bytes on disk
/// show it.
#[test]
fn reordering_persists_the_graph_it_renumbered() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    {
        let db = Database::open(dir.path()).expect("test: open");
        db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
            .expect("test: create");
        let collection = db.get_vector_collection("docs").expect("test: collection");
        collection
            .upsert(
                (0..POINTS)
                    .map(|id| Point::new(id, vector(id), None))
                    .collect::<Vec<_>>(),
            )
            .expect("test: upsert");
        collection.flush_full().expect("test: flush");
    }

    let vectors = find(dir.path(), "vectors");
    let graph = find(dir.path(), "graph");

    let db = Database::open(dir.path()).expect("test: reopen");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let (vectors_before, graph_before) = (digest(&vectors), digest(&graph));

    collection.reorder_for_locality().expect("test: reorder");

    let (vectors_after, graph_after) = (digest(&vectors), digest(&graph));
    assert_ne!(
        vectors_before, vectors_after,
        "CONTROL: the permutation must have reached the durable store, or this \
         fixture never triggered a reorder and the assertion below is vacuous"
    );
    assert_ne!(
        graph_before, graph_after,
        "the durable vectors were permuted but `.graph` still carries the old \
         numbering: a crash here resolves every node id to the wrong vector"
    );
}

/// And the collection still answers correctly afterwards.
///
/// The complement: a `reorder` that corrupted the index while dutifully saving
/// both files would satisfy the test above.
#[test]
fn a_reordered_collection_still_finds_its_nearest_neighbour() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let db = Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    collection
        .upsert(
            (0..POINTS)
                .map(|id| Point::new(id, vector(id), None))
                .collect::<Vec<_>>(),
        )
        .expect("test: upsert");
    collection.flush_full().expect("test: flush");

    let probe = 1_234_u64;
    let before = collection
        .search(&vector(probe), 1)
        .expect("test: search before")
        .first()
        .map(|r| r.point.id);
    assert_eq!(
        before,
        Some(probe),
        "CONTROL: the fixture must be findable at all"
    );

    collection.reorder_for_locality().expect("test: reorder");

    assert_eq!(
        collection
            .search(&vector(probe), 1)
            .expect("test: search after")
            .first()
            .map(|r| r.point.id),
        Some(probe),
        "the reorder renumbered the graph out from under the vectors"
    );
}
