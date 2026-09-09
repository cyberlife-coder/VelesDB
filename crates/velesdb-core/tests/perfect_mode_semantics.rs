#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)] // ids into f32 coordinates, deliberately
#![allow(clippy::cast_possible_truncation)] // usize ids into u64, on a 200-point fixture

//! What `Perfect` means on each of the two axes, pinned.
//!
//! Both axes carry a variant named `Perfect` and, until #2238, both described
//! it wrongly. `SearchQuality::Perfect` was documented as staying on the graph
//! at `ef_search = 4096`; it leaves the graph entirely. `SearchMode::Perfect`
//! maps to an `ef_search` of `usize::MAX` commented "Signals bruteforce", which
//! nothing reads as a signal.
//!
//! Both errors were made the same way — reading `ef_search()` without checking
//! which arm of `try_search_special_quality` runs — so the corrections are
//! pinned by behaviour rather than restated in prose.

use velesdb_core::{Database, DistanceMetric, Point, SearchMode, SearchQuality, VelesConfig};

const DIM: usize = 8;
const POINTS: usize = 200;
const CAP: usize = 10;

const _: () = assert!(
    CAP < POINTS,
    "the cap must sit below the collection size or the guard rail cannot fire"
);

fn collection_with_cap(dir: &tempfile::TempDir, cap: usize) -> velesdb_core::VectorCollection {
    let mut config = VelesConfig::default();
    config.limits.max_perfect_mode_vectors = cap;
    let db = Database::open_with_config(dir.path(), config).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::new(id as u64, vec![id as f32; DIM], None))
        .collect();
    collection.upsert(points).expect("test: upsert");
    collection
}

/// `SearchQuality::Perfect` is exhaustive, and the cap that says so refuses it.
///
/// The two other arms are the control: without them a collection that refused
/// *every* search would satisfy the first assertion. `ef = 4096` is the
/// specific control that matters, because it is the number the old doc claimed
/// `Perfect` runs at — it is accepted, so the two are demonstrably not the same
/// path.
#[test]
fn perfect_quality_is_refused_above_the_configured_cap() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = collection_with_cap(&dir, CAP);
    let query = vec![7.0_f32; DIM];

    let refused = collection
        .search_with_quality(&query, 5, SearchQuality::Perfect)
        .expect_err("Perfect above the cap must be refused");
    let message = refused.to_string();
    assert!(
        message.contains("max_perfect_mode_vectors"),
        "the refusal must name the knob that caused it, got: {message}"
    );

    assert!(
        collection
            .search_with_quality(&query, 5, SearchQuality::Accurate)
            .is_ok(),
        "only Perfect is capped; Accurate must still run"
    );
    assert!(
        collection.search_with_ef(&query, 5, 4096).is_ok(),
        "ef = 4096 is not Perfect: it stays on the graph and is not capped"
    );
}

/// Raising the cap above the collection size lets the same call through.
///
/// Without this, the test above could pass on a collection that simply cannot
/// answer a Perfect search at all.
#[test]
fn perfect_quality_runs_once_the_cap_allows_it() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = collection_with_cap(&dir, POINTS + 1);
    let hits = collection
        .search_with_quality(&[7.0_f32; DIM], 5, SearchQuality::Perfect)
        .expect("test: Perfect under the cap must run");
    assert_eq!(hits.len(), 5, "an exhaustive scan still returns k results");
    assert_eq!(
        hits.first().map(|r| r.point.id),
        Some(7),
        "and it returns the exact nearest neighbour"
    );
}

/// `SearchMode::quality()` reaches the guarded variant; `ef_search()` cannot.
///
/// This is the conversion the `[search]` wiring (#2087) must use. Routed
/// through `ef_search()` instead, `perfect` becomes `Custom(usize::MAX)` —
/// accepted by a collection the cap should have protected, which is the silent
/// downgrade this asserts against.
#[test]
fn search_mode_perfect_survives_only_the_lossless_conversion() {
    assert_eq!(SearchMode::Perfect.quality(), SearchQuality::Perfect);
    assert_eq!(SearchMode::Fast.quality(), SearchQuality::Fast);
    assert_eq!(SearchMode::Balanced.quality(), SearchQuality::Balanced);
    assert_eq!(SearchMode::Accurate.quality(), SearchQuality::Accurate);

    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = collection_with_cap(&dir, CAP);
    let query = vec![7.0_f32; DIM];

    assert!(
        collection
            .search_with_quality(&query, 5, SearchMode::Perfect.quality())
            .is_err(),
        "via quality(), the mode reaches Perfect and the cap refuses it"
    );
    assert!(
        collection
            .search_with_ef(&query, 5, SearchMode::Perfect.ef_search())
            .is_ok(),
        "via ef_search(), it does not: an uncapped traversal the guard never sees. \
         This assertion documents the loss -- flip it only by removing the loss."
    );
}
