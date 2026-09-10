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

fn reopened(
    dir: &tempfile::TempDir,
    config: VelesConfig,
) -> (Database, velesdb_core::VectorCollection) {
    let db = Database::open_with_config(dir.path(), config).expect("test: reopen");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    (db, collection)
}

/// A per-query `ef` does not depend on the configured default.
///
/// One persisted index, reopened under two DIFFERENT configured defaults, must
/// give the same answer to the same explicit call -- if the default leaked into
/// explicit searches the two would disagree. Same index on purpose: two indexes
/// built separately can differ by construction and would make the equality
/// flap for reasons unrelated to configuration.
///
/// The first draft compared an explicit call to ITSELF and labelled it a
/// control, which could not fail, and asserted only that explicit and default
/// answers differ, which a clamped `ef` satisfies (#2246, P2-b and P2-e). That
/// an explicit `ef` is honoured at all is proven by the control of
/// `a_configured_ef_search_reaches_an_unqualified_search`, which fails if
/// `search_with_ef` ignores its argument.
#[test]
fn a_per_query_ef_is_independent_of_the_configured_default() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    seeded(&dir, VelesConfig::default())
        .flush_full()
        .expect("test: persist the index both opens will read");
    let query = vector(POINTS as u64 + 1);

    let (db_low, low) = reopened(&dir, config_with_ef(LOW_EF));
    let low_default = ids(&low.search(&query, K).expect("test: low default"));
    let low_explicit = ids(&low
        .search_with_ef(&query, K, HIGH_EF)
        .expect("test: explicit"));
    drop(low);
    drop(db_low);

    let (_db_high, high) = reopened(&dir, config_with_ef(HIGH_EF));
    let high_default = ids(&high.search(&query, K).expect("test: high default"));
    let high_explicit = ids(&high
        .search_with_ef(&query, K, HIGH_EF)
        .expect("test: explicit"));

    assert_ne!(
        low_default, high_default,
        "CONTROL: the two configured defaults must answer differently, or the \
         equality below could not tell a leak from a default with no effect"
    );
    assert_eq!(
        low_explicit, high_explicit,
        "an explicit ef must give the same answer whatever default the collection was opened with"
    );
}

/// `perfect` as a global default still loads, and is applied as `accurate`.
///
/// This refused at load until the seven-lens review (#2246): a TOML accepted by
/// v6.0.0 then failed `Database::open`, a breaking change shipped under
/// `### Added`. The concern behind the refusal is kept -- a filtered search's
/// bitmap pre-filter never reads the configured quality, so a global `Perfect`
/// would scan on some queries and traverse on others -- by never letting the
/// default resolve to it.
///
/// Asserted on the resolution because
/// `a_configured_ef_search_reaches_an_unqualified_search` already proves the
/// resolved quality is what `search()` runs; together they cover the path.
#[test]
fn perfect_as_a_global_default_still_opens_and_resolves_to_accurate() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let mut opening = VelesConfig::default();
    opening.search.default_mode = SearchMode::Perfect;
    Database::open_with_config(dir.path(), opening)
        .expect("a config v6.0.0 accepted must keep opening a database");

    let mut config = VelesConfig::default();
    config.search.default_mode = SearchMode::Perfect;
    assert_eq!(
        config.search.resolved_quality(),
        velesdb_core::SearchQuality::Accurate,
        "a global `perfect` must resolve to `accurate`: the bitmap pre-filter never reads it"
    );

    config.search.default_mode = SearchMode::Balanced;
    assert_eq!(
        config.search.resolved_quality(),
        velesdb_core::SearchQuality::Balanced,
        "CONTROL: only `perfect` is downgraded; every other mode resolves to itself"
    );
}
