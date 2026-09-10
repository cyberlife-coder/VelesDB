#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)] // ids into f32 coordinates, deliberately
#![allow(clippy::cast_possible_truncation)] // usize ids into u64, on a 3 000-point fixture

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

const DIM: usize = 32;
const POINTS: usize = 3_000;
const K: usize = 10;
const LOW_EF: usize = 16;
/// Queries the CONTROL is taken over: one could come out exact by chance.
const QUERIES: usize = 50;
const CAP: usize = 10;

const _: () = assert!(
    CAP < POINTS,
    "the cap must sit below the collection size or the guard rail cannot fire"
);

/// Deterministic, dispersed coordinates.
///
/// The first fixture was `vec![id as f32; DIM]`: every vector on one line, so
/// the nearest neighbour of any query is the same for every search mode and an
/// "exact" assertion could not tell a brute force from a graph (#2246, P2-e).
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

/// The true top-`k` by Euclidean distance, computed here without the engine.
fn exact_top_k(query: &[f32], k: usize) -> Vec<u64> {
    let mut scored: Vec<(f32, u64)> = (0..POINTS as u64)
        .map(|id| {
            let d: f32 = vector(id)
                .iter()
                .zip(query)
                .map(|(a, b)| (a - b) * (a - b))
                .sum();
            (d, id)
        })
        .collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    scored.into_iter().take(k).map(|(_, id)| id).collect()
}

fn collection_with_cap(dir: &tempfile::TempDir, cap: usize) -> velesdb_core::VectorCollection {
    let mut config = VelesConfig::default();
    config.limits.max_perfect_mode_vectors = cap;
    let db = Database::open_with_config(dir.path(), config).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let points: Vec<Point> = (0..POINTS)
        .map(|id| Point::new(id as u64, vector(id as u64), None))
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

/// Raising the cap lets the call through, and what comes back is EXACT.
///
/// The first version asserted `top1 == 7` on a collinear fixture, which every
/// mode satisfies: routing `Perfect` to a low-ef graph search would have left it
/// green (#2246, P2-e). The answer is now compared with a ground truth computed
/// here, after a CONTROL shows a low-ef graph search misses on this fixture —
/// on at least one of `QUERIES` queries, not on one: `search_with_ef` reranks
/// `4 × k` candidates exactly and the graph is built by a parallel insert, so a
/// single query can come out exact by chance and turn the control red.
#[test]
fn perfect_quality_runs_once_the_cap_allows_it_and_is_exact() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = collection_with_cap(&dir, POINTS + 1);
    let ids = |hits: Vec<velesdb_core::SearchResult>| {
        hits.into_iter().map(|r| r.point.id).collect::<Vec<_>>()
    };

    let mut graph_misses = 0;
    for i in 0..QUERIES {
        let query = vector((POINTS + 1 + i) as u64);
        let truth = exact_top_k(&query, K);
        let graph = ids(collection
            .search_with_ef(&query, K, LOW_EF)
            .expect("test: low-ef graph search"));
        if graph != truth {
            graph_misses += 1;
        }
        let perfect = ids(collection
            .search_with_quality(&query, K, SearchQuality::Perfect)
            .expect("test: Perfect under the cap must run"));
        assert_eq!(
            perfect, truth,
            "Perfect must return the exact top-k, in order"
        );
    }
    assert!(
        graph_misses > 0,
        "CONTROL: a low-ef graph search must miss on this fixture — on at least one of \
         {QUERIES} queries — or the exactness asserted above would prove nothing about Perfect"
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
