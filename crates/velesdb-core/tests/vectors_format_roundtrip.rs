#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)]
// ids into f32 coordinates, deliberately

// Without `persistence` there is no `Database`, no mapped arena and no
// `.vectors` at all. Most files in this directory carry the same gate. It is
// not decoration: `quality-deep.yml` runs `cargo miri test
// --no-default-features -p velesdb-core` WITHOUT `--lib`, which compiles every
// test target - so an ungated file breaks a job that only runs on Sundays.

//! End-to-end coverage of `{basename}.vectors` across a real database lifecycle.
//!
//! # Why these assertions read the FILE and not the collection
//!
//! An earlier version of this file asserted through `Collection` — `len()` and
//! a handful of `search()` probes — and could not fail. Measured: delete
//! `native_hnsw.vectors` outright, reopen, and `len()` still answers 64 while
//! `search()` still returns the right id. The gap recovery rebuilds the graph
//! from `vectors.idx` and the WAL at open, so **no assertion reaching through
//! the collection API observes `.vectors` at all**.
//!
//! Everything below therefore reads the file: its header, its payload bytes,
//! and its size. That is the only surface on which a claim about this format
//! can be made.
//!
//! # What each test is for
//!
//! 1. **Opening never writes.** Checked on both sides of the arena capacity
//!    floor, because the regression that motivated this file came from the
//!    side *below* it: adoption there would size the arena up to the floor and
//!    extend the file, and `velesdb-memory`'s migration resume proves a source
//!    store unchanged by hashing exactly these files.
//! 2. **Growing an adopted arena persists every vector.** The sequence no
//!    other test performs: reopen (which maps the file), insert past the
//!    persisted count so the mapping must grow, save — which rewrites the
//!    header *in place* on a file the arena still maps — then read every f32
//!    back out of the file.
//! 3. **The grown arena was adopted, not copied.** Without this the file above
//!    could be exercising the copy path on both sides and prove nothing about
//!    mapping. The witness is public even though `backing_path` is not: an
//!    adopted arena grows its capacity by an eighth, past a batch smaller than
//!    that, so the file ends up larger than the payload it declares, while the
//!    copy path writes an exact fit.
//! 4. **A disposable arena's owner never deletes the durable store.** Run under
//!    `StorageMode::SQ8`, because `Full` never constructs an `ArenaHome` at all
//!    and the assertion would pass without ever reaching the hazard.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point, StorageMode};

/// Payload offset of a v2 `.vectors`, mirrored from `graph_io.rs`.
///
/// Duplicated rather than imported because this is an integration test: it
/// links the crate from outside, where the constant is `pub(crate)`. The header
/// assertion below refuses any version but 2, so a format that moves this
/// offset makes these tests fail loudly rather than read the wrong bytes.
const DATA_OFFSET: u64 = 4096;
/// Header length of every `.vectors` version — version `u32`, count `u64`,
/// dimension `u32` — mirrored from `graph_io.rs` for the same reason.
const HEADER_BYTES: usize = 16;
/// Width of one stored value: the payload is little-endian `f32`.
const VALUE_BYTES: usize = std::mem::size_of::<f32>();
const DIM: usize = 8;

/// Comfortably above the arena capacity floor, so adoption applies.
const ADOPTED: u64 = 64;
/// Comfortably below it, so the copy path applies.
const SMALL: u64 = 5;
/// Enough new points to force the mapping to grow past its persisted capacity.
const EXTRA: u64 = 4;

// The size witness below reads "the file is larger than the payload it
// declares". A file-backed arena grows to `max(needed, capacity + capacity / 8)`
// (#2246). Below that first eighth, the grown capacity OVERSHOOTS the final
// count whether the upsert reserves its batch at once or grows push by push, as
// this test's does. From the eighth on it can land exactly on the count —
// always under a batch reservation, and push by push whenever the count is a
// growth step (72 here, at EXTRA == ADOPTED / 8) — and the witness would then
// report "copied" on an arena that was in fact adopted: a false negative with a
// confident message. The constraint lives here, where changing a constant trips
// it, rather than in a sentence nobody re-reads.
const _: () = assert!(
    EXTRA < ADOPTED / 8,
    "the adoption witness needs the grown capacity to overshoot the final count"
);

fn make_vector(id: u64) -> Vec<f32> {
    (0..DIM)
        .map(|i| ((id as f32) * 0.37 + (i as f32) * 0.11).sin())
        .collect()
}

fn points(range: std::ops::Range<u64>) -> Vec<Point> {
    range
        .map(|id| Point::new(id, make_vector(id), Some(json!({ "id": id }))))
        .collect()
}

/// Finds `native_hnsw.vectors` under the database directory.
///
/// Located by walking rather than by reconstructing the layout: a test that
/// hard-codes the path keeps passing when the layout moves, silently hashing a
/// file that is no longer the one under test.
fn vectors_file(root: &Path) -> PathBuf {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("test: read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == "native_hnsw.vectors") {
                found.push(path);
            }
        }
    }
    assert_eq!(
        found.len(),
        1,
        "test: expected exactly one native_hnsw.vectors under {}, found {found:?}",
        root.display()
    );
    found.pop().expect("test: checked non-empty just above")
}

/// `(count, dimension)` from the header, refusing any version but 2.
///
/// The version check is not ceremony: a v3 that moves a field would otherwise
/// be decoded as if it were v2, and every assertion below would compare
/// confidently against the wrong bytes.
fn header(path: &Path) -> (u64, u32) {
    let bytes = std::fs::read(path).expect("test: read .vectors");
    assert!(
        bytes.len() >= usize::try_from(DATA_OFFSET).expect("test: offset fits a usize"),
        "test: file shorter than its own header region"
    );
    let version = u32::from_le_bytes(bytes[0..4].try_into().expect("test: 4 bytes"));
    assert_eq!(version, 2, "test: these assertions decode v2 only");
    let count = u64::from_le_bytes(bytes[4..12].try_into().expect("test: 8 bytes"));
    let dimension = u32::from_le_bytes(bytes[12..HEADER_BYTES].try_into().expect("test: 4 bytes"));
    (count, dimension)
}

/// Every vector the file declares, decoded from its payload.
///
/// This is the assertion surface the collection API cannot provide. Reads
/// exactly `count * dimension` values from `DATA_OFFSET`; anything the arena
/// left beyond them is uncommitted capacity the reader is meant to ignore.
fn payload(path: &Path) -> Vec<Vec<f32>> {
    let (count, dimension) = header(path);
    let bytes = std::fs::read(path).expect("test: read .vectors");
    let dim = dimension as usize;
    let base = usize::try_from(DATA_OFFSET).expect("test: offset fits a usize");
    let stored = usize::try_from(count).expect("test: count fits a usize");
    let needed = base + stored * dim * VALUE_BYTES;
    assert!(
        bytes.len() >= needed,
        "test: file holds {} bytes but its header declares {needed}",
        bytes.len()
    );
    (0..stored)
        .map(|v| {
            (0..dim)
                .map(|k| {
                    let at = base + (v * dim + k) * VALUE_BYTES;
                    f32::from_le_bytes(
                        bytes[at..at + VALUE_BYTES]
                            .try_into()
                            .expect("test: 4 bytes"),
                    )
                })
                .collect()
        })
        .collect()
}

/// Every disposable arena file under `root`.
///
/// `ArenaHome::claim` names them `hnsw-{token}.arena`, so their presence is a
/// public, on-disk witness that a graph really owns one — no `pub(crate)`
/// access needed, and no property of the growth strategy inferred.
fn disposable_arenas(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("test: read dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "arena") {
                found.push(path);
            }
        }
    }
    found
}

fn digest(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    std::fs::read(path)
        .expect("test: read .vectors")
        .hash(&mut hasher);
    hasher.finish()
}

/// Creates a collection holding `ids`, then closes the database.
fn seed(dir: &TempDir, ids: std::ops::Range<u64>, mode: StorageMode) {
    seed_with(dir, ids, mode, DistanceMetric::Euclidean);
}

/// [`seed`], with the metric chosen by the caller.
fn seed_with(dir: &TempDir, ids: std::ops::Range<u64>, mode: StorageMode, metric: DistanceMetric) {
    let db = Database::open(dir.path()).expect("test: open database");
    db.create_vector_collection_with_options("docs", DIM, metric, mode)
        .expect("test: create collection");
    let collection = db
        .get_vector_collection("docs")
        .expect("test: collection exists");
    collection.upsert(points(ids)).expect("test: upsert");
    collection.flush_full().expect("test: flush");
}

/// One way of making a `.vectors` unusable, with the label a failure reports.
type Mutilation = (&'static str, fn(&Path));

/// An unusable `.vectors` must not prevent a collection from opening.
///
/// This is the property the v2 release note rests on. The note first claimed
/// the opposite -- that an older build "will not open a database written by
/// this one" -- read off the v1 reader, which does refuse a v2 header with
/// `Unsupported version: 2`, without checking that anything reaches it.
/// Measured against a real `v6.0.0` build: it opens a v2 database and returns
/// the correct nearest neighbour. `.vectors` is a derived artifact, and the
/// collection is rebuilt from `vectors.dat` / the WAL when it cannot be used.
///
/// The three mutilations are the ones that experiment ran, and each fails a
/// different way: an unknown version is rejected by the header check, a
/// header-only file passes the version check and then runs out of payload, and
/// an absent file never opens at all. A test that only deleted the file would
/// leave both read failures unexercised.
///
/// The control is the untouched arm: it asserts the same count from the same
/// seed, so a harness that answered 0 everywhere could not pass.
#[test]
fn a_corrupt_vectors_file_does_not_prevent_opening() {
    let mutilate: [Mutilation; 4] = [
        ("intact (control)", |_| {}),
        ("unknown version", |p| {
            let mut bytes = std::fs::read(p).expect("test: read");
            bytes[0..4].copy_from_slice(&99u32.to_le_bytes());
            std::fs::write(p, bytes).expect("test: write");
        }),
        ("header only", |p| {
            let bytes = std::fs::read(p).expect("test: read");
            std::fs::write(p, &bytes[..HEADER_BYTES]).expect("test: truncate");
        }),
        ("absent", |p| std::fs::remove_file(p).expect("test: remove")),
    ];

    for (label, damage) in mutilate {
        let dir = TempDir::new().expect("test: tempdir");
        seed(&dir, 0..ADOPTED, StorageMode::Full);
        damage(&vectors_file(dir.path()));

        let db = Database::open(dir.path()).unwrap_or_else(|e| panic!("{label}: open failed: {e}"));
        let collection = db
            .get_vector_collection("docs")
            .unwrap_or_else(|| panic!("{label}: collection missing"));
        assert_eq!(
            collection.len(),
            usize::try_from(ADOPTED).expect("test: count fits a usize"),
            "{label}: an unusable .vectors must not cost the collection its points"
        );
        let hit = collection
            .search(&make_vector(7), 1)
            .unwrap_or_else(|e| panic!("{label}: search failed: {e}"));
        assert_eq!(
            hit.first().map(|r| r.point.id),
            Some(7),
            "{label}: the rebuilt index must still answer correctly"
        );
    }
}

/// Opening a collection must never write to its `.vectors`, on either side of
/// the arena capacity floor.
///
/// Below the floor adoption is refused and the copy path runs; above it the
/// file is mapped. Both must leave the bytes untouched. The regression this
/// guards took out twenty-one `velesdb-memory` tests when adoption first
/// shipped: its migration resume proves a source store unchanged by hashing
/// these very files, so a store that grew on open made a correct resume look
/// like a corrupted one.
///
/// Run under Cosine as well as Euclidean since #2246 (P2-f). The load path
/// renormalises a cosine collection's vectors in place, on an arena that IS the
/// durable file once adopted. It writes nothing today only because the vectors
/// were normalised at insert, inside the load's 1e-5 tolerance. Only the
/// (Cosine, ADOPTED) arm can notice if that stops being true: below the
/// capacity floor the copy path normalises a heap copy, never the file.
#[test]
fn opening_a_collection_never_writes_to_its_vectors_file() {
    for (metric, count) in [
        (DistanceMetric::Euclidean, SMALL),
        (DistanceMetric::Euclidean, ADOPTED),
        (DistanceMetric::Cosine, SMALL),
        (DistanceMetric::Cosine, ADOPTED),
    ] {
        let dir = TempDir::new().expect("test: tempdir");
        seed_with(&dir, 0..count, StorageMode::Full, metric);
        let file = vectors_file(dir.path());
        let before = digest(&file);

        {
            let db = Database::open(dir.path()).expect("test: reopen");
            let collection = db
                .get_vector_collection("docs")
                .expect("test: collection exists");
            assert_eq!(
                collection.len(),
                usize::try_from(count).expect("test: count fits a usize"),
                "test: the fixture did not load at {count} points under {metric:?}"
            );
        }

        assert_eq!(
            digest(&file),
            before,
            "test: opening a collection wrote to its .vectors at {count} points under {metric:?}"
        );
    }
}

/// Growing an adopted arena and saving persists every vector to the file.
///
/// This is the sequence `File::create` would have broken: the dump rewrites the
/// header in place on a file the arena still maps, because truncating it would
/// remove the pages the mapping points at. The failure mode there is a SIGBUS
/// rather than an assertion, so what a test can do is prove the bytes are right
/// afterwards — which is why every vector is read back out of the file rather
/// than out of the collection.
#[test]
fn growing_an_adopted_arena_persists_every_vector_to_the_file() {
    let dir = TempDir::new().expect("test: tempdir");
    seed(&dir, 0..ADOPTED, StorageMode::Full);
    let file = vectors_file(dir.path());
    assert_eq!(header(&file).0, ADOPTED, "test: the seed did not persist");

    {
        let db = Database::open(dir.path()).expect("test: reopen");
        let collection = db
            .get_vector_collection("docs")
            .expect("test: collection exists");
        collection
            .upsert(points(ADOPTED..ADOPTED + EXTRA))
            .expect("test: upsert past the persisted count");
        collection.flush_full().expect("test: flush");
    }

    let total = ADOPTED + EXTRA;
    assert_eq!(
        header(&file).0,
        total,
        "test: the in-place header rewrite did not record the grown count"
    );

    let stored = payload(&file);
    assert_eq!(stored.len(), usize::try_from(total).expect("test: fits"));
    for (id, vector) in stored.iter().enumerate() {
        assert_eq!(
            vector.as_slice(),
            make_vector(id as u64).as_slice(),
            "test: vector {id} did not survive the grow-and-save"
        );
    }
}

/// The grown arena was adopted, not copied.
///
/// Without this, the test above could be exercising the copy path on both sides
/// and would prove nothing about mapping. `backing_path` is `pub(crate)` and
/// out of reach here, but the consequence is public: an adopted arena grows its
/// capacity by an eighth — past `EXTRA`, which is kept below that eighth — so
/// the file ends up **larger** than the payload it declares. The copy path
/// writes an exact fit.
#[test]
fn a_grown_arena_was_adopted_rather_than_copied() {
    let dir = TempDir::new().expect("test: tempdir");
    seed(&dir, 0..ADOPTED, StorageMode::Full);
    let file = vectors_file(dir.path());

    {
        let db = Database::open(dir.path()).expect("test: reopen");
        let collection = db
            .get_vector_collection("docs")
            .expect("test: collection exists");
        collection
            .upsert(points(ADOPTED..ADOPTED + EXTRA))
            .expect("test: upsert");
        collection.flush_full().expect("test: flush");
    }

    let (count, dimension) = header(&file);
    let value_bytes = u64::try_from(VALUE_BYTES).expect("test: a value width fits u64");
    let exact_fit = DATA_OFFSET + count * u64::from(dimension) * value_bytes;
    let actual = std::fs::metadata(&file).expect("test: stat").len();
    assert!(
        actual > exact_fit,
        "test: the file is an exact fit ({actual} bytes) — the arena was copied, \
         not adopted, so the mapped path this suite targets never ran"
    );
}

/// A collection whose arena is disposable must still keep its durable store.
///
/// `ArenaHome::drop` removes its file unconditionally, and that is correct: the
/// disposable arena is a cache. The hazard is pointing it at `.vectors`.
///
/// Run under `SQ8` on purpose. `Full` never constructs an `ArenaHome` at all,
/// so the same assertion there passes without the hazard ever being reached —
/// a green test about nothing.
#[test]
fn dropping_a_collection_with_a_disposable_arena_keeps_its_vectors() {
    let dir = TempDir::new().expect("test: tempdir");
    seed(&dir, 0..SMALL, StorageMode::SQ8);
    let file = vectors_file(dir.path());
    let before = digest(&file);

    let db = Database::open(dir.path()).expect("test: reopen");
    let collection = db
        .get_vector_collection("docs")
        .expect("test: collection exists");
    assert_eq!(
        collection.len(),
        usize::try_from(SMALL).expect("test: fits"),
        "test: the fixture did not load"
    );
    assert!(
        !disposable_arenas(dir.path()).is_empty(),
        "test: no hnsw-*.arena exists, so this collection never owned a disposable \
         arena and the drop hazard is never reached"
    );
    drop(db);

    assert!(
        file.exists(),
        "test: closing a collection deleted its durable vector store"
    );
    assert_eq!(
        digest(&file),
        before,
        "test: closing a collection rewrote its durable vector store"
    );
}

// Deliberately NOT asserted above: that the disposable arena file is gone after
// the drop. It is not, and that is not a defect — `ArenaHome::sweep_stale`
// exists precisely because "a crash or a kill skips Drop, so a collection
// directory can carry arenas from a previous run", and the sweep runs at the
// next collection open. An arena outliving its owner is an anticipated state,
// so asserting its absence would pin a promise the design does not make. The
// hazard this test guards is the other one: that the drop takes `.vectors`
// with it.
