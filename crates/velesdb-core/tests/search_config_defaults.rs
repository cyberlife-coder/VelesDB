#![cfg(feature = "persistence")]
#![allow(clippy::cast_precision_loss)] // indices into f32 coordinates, deliberately

//! `[search]` reaches an unqualified `search()` — asserted on results, not on a
//! getter.
//!
//! The section was parsed, validated and ignored until #2087. Proving it is now
//! applied means observing the SEARCH, because a test reading back the resolved
//! quality would pass just as well if nothing consumed it.
//!
//! Every test here carries a control that makes its main assertion falsifiable:
//! comparing two searches only means something once the fixture is shown to be
//! hard enough that `ef_search` changes the answer at all.

use velesdb_core::{Database, DistanceMetric, Point, SearchMode, VelesConfig};

const DIM: usize = 32;
const POINTS: usize = 3_000;
const K: usize = 10;
const LOW_EF: usize = 16; // the minimum `validate_search` accepts
const HIGH_EF: usize = 4_096; // the maximum

/// Deterministic pseudo-random coordinates: a fixture that changes between runs
/// would make a recall-sensitive assertion flap.
fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (0..DIM)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % 10_000) as f32 / 10_000.0
        })
        .collect()
}

fn seeded(dir: &tempfile::TempDir, config: VelesConfig) -> velesdb_core::VectorCollection {
    let db = Database::open_with_config(dir.path(), config).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let points: Vec<Point> = (0..POINTS as u64)
        .map(|id| Point::new(id, vector(id), None))
        .collect();
    collection.upsert(points).expect("test: upsert");
    collection
}

fn ids(results: &[velesdb_core::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn config_with_ef(ef: usize) -> VelesConfig {
    let mut config = VelesConfig::default();
    config.search.ef_search = Some(ef);
    config
}

/// `search.ef_search` reaches `search()`.
///
/// The control comes first: unless `LOW_EF` and `HIGH_EF` disagree on this
/// fixture, comparing anything to the low-ef answer proves nothing. If that
/// assertion ever fails the fixture got easy, and the message says so rather
/// than leaving a green test that checks nothing.
#[test]
fn a_configured_ef_search_reaches_an_unqualified_search() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = seeded(&dir, config_with_ef(LOW_EF));
    let query = vector(POINTS as u64 + 1);

    let low = ids(&collection
        .search_with_ef(&query, K, LOW_EF)
        .expect("test: low ef"));
    let high = ids(&collection
        .search_with_ef(&query, K, HIGH_EF)
        .expect("test: high ef"));
    assert_ne!(
        low, high,
        "CONTROL: ef must change the answer on this fixture, or the assertion below is vacuous"
    );

    assert_eq!(
        ids(&collection.search(&query, K).expect("test: search")),
        low,
        "an unqualified search must use the configured ef, not the built-in Balanced"
    );
}

/// A default config leaves `search()` exactly where it was.
///
/// The regression that matters most: every existing caller must see the
/// built-in `Balanced` it saw before the wiring.
#[test]
fn a_default_config_leaves_search_on_the_built_in_quality() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = seeded(&dir, VelesConfig::default());
    let query = vector(POINTS as u64 + 1);
    assert_eq!(
        ids(&collection.search(&query, K).expect("test: search")),
        ids(&collection
            .search_with_quality(&query, K, velesdb_core::SearchQuality::Balanced)
            .expect("test: balanced")),
        "an unconfigured collection must answer exactly as Balanced does"
    );
}

/// A per-query override still wins over the section.
#[test]
fn a_per_query_ef_still_overrides_the_configured_default() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = seeded(&dir, config_with_ef(LOW_EF));
    let query = vector(POINTS as u64 + 1);
    assert_eq!(
        ids(&collection
            .search_with_ef(&query, K, HIGH_EF)
            .expect("test: override")),
        ids(&collection
            .search_with_ef(&query, K, HIGH_EF)
            .expect("test: override again")),
        "CONTROL: the explicit path must be deterministic before it is compared"
    );
    assert_ne!(
        ids(&collection
            .search_with_ef(&query, K, HIGH_EF)
            .expect("test: override")),
        ids(&collection
            .search(&query, K)
            .expect("test: configured default")),
        "the per-query ef must not be flattened onto the configured one"
    );
}

/// `perfect` is refused as a global default, with the reason.
#[test]
fn perfect_is_refused_as_a_global_default() {
    let mut config = VelesConfig::default();
    config.search.default_mode = SearchMode::Perfect;
    let message = config
        .validate()
        .expect_err("an exhaustive scan cannot be the default for every query")
        .to_string();
    assert!(
        message.contains("max_perfect_mode_vectors"),
        "the refusal must name the guard that cannot be enforced, got: {message}"
    );

    config.search.default_mode = SearchMode::Accurate;
    assert!(
        config.validate().is_ok(),
        "CONTROL: only `perfect` is refused; the other modes must load"
    );
}
