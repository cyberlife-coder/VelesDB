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

use std::collections::HashMap;
use velesdb_core::{Database, DistanceMetric, Point, SearchMode, VelesConfig};

const DIM: usize = 32;
const POINTS: usize = 3_000;
// The corpus for the tests whose paths overfetch candidates (rerank-only
// `WITH`, batch, multi-query). At `POINTS` that candidate pool recovers the
// exact top-k at `LOW_EF` as reliably as at `HIGH_EF`, so comparing the two
// configured defaults proves nothing. Even at this size a single query is not
// enough: the level RNG is seeded with a constant, but parallel insert
// threads draw from it in a nondeterministic order, so each build is a
// different graph and one query can land where `LOW_EF` is already exact on
// one run and not on the next, with identical code and data (seen with
// `--test-threads=1` too). Searches on one built graph are repeatable. Those
// tests therefore run `QUERIES` vectors and need only one to disagree. For
// the batch test a reverted fix runs the same `Balanced` under both configs,
// so every query agrees on every run. For the rerank-only test a revert
// answers at `Balanced`, which can equal the `LOW_EF` answer, so a
// revert fails only where `Balanced` and `LOW_EF` differ on a query where
// `LOW_EF` and `HIGH_EF` differ: measured to happen, not guaranteed by shape.
const HARD_POINTS: usize = 60_000;
const K: usize = 10;
// `k` for `search_batch_with_filters` and `multi_query_search`; see
// `batch_answers`.
const WIDE_K: usize = 50;
const MULTI_K: usize = 101;
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
    seeded_n(dir, config, POINTS)
}

fn seeded_n(
    dir: &tempfile::TempDir,
    config: VelesConfig,
    n: usize,
) -> velesdb_core::VectorCollection {
    seeded_with(dir, config, n, |_| None)
}

/// `HARD_POINTS` points, each carrying `cat = id % 2` for the filtered path.
fn seeded_hard(dir: &tempfile::TempDir, config: VelesConfig) -> velesdb_core::VectorCollection {
    seeded_with(dir, config, HARD_POINTS, |id| {
        Some(serde_json::json!({ "cat": id % 2 }))
    })
}

fn seeded_with(
    dir: &tempfile::TempDir,
    config: VelesConfig,
    n: usize,
    payload: impl Fn(u64) -> Option<serde_json::Value>,
) -> velesdb_core::VectorCollection {
    let db = Database::open_with_config(dir.path(), config).expect("test: open");
    db.create_vector_collection("docs", DIM, DistanceMetric::Euclidean)
        .expect("test: create");
    let collection = db.get_vector_collection("docs").expect("test: collection");
    let points: Vec<Point> = (0..n as u64)
        .map(|id| Point::new(id, vector(id), payload(id)))
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

const QUERIES: u64 = 8;

fn hard_queries() -> Vec<Vec<f32>> {
    (0..QUERIES)
        .map(|i| vector(HARD_POINTS as u64 + 1 + i))
        .collect()
}

/// The ids a `NEAR` query returns, optionally through the `cat = 0` metadata
/// filter the hard fixture carries. An empty `with_clause` omits `WITH`.
fn with_query_ids(
    collection: &velesdb_core::VectorCollection,
    query: &[f32],
    filtered: bool,
    with_clause: &str,
) -> Vec<u64> {
    let filter = if filtered { "AND cat = 0 " } else { "" };
    let with = if with_clause.is_empty() {
        String::new()
    } else {
        format!(" WITH ({with_clause})")
    };
    let sql = format!("SELECT * FROM docs WHERE vector NEAR $v {filter}LIMIT {K}{with}");
    let mut params = HashMap::new();
    params.insert("v".to_string(), serde_json::json!(query));
    ids(&collection
        .execute_query_str(&sql, &params)
        .expect("test: WITH query"))
}

/// `WITH ({option})` answers exactly like `WITH (ef_search = N, {option})`
/// where `N` is the configured `[search]` ef; an empty `option` is a query
/// with no `WITH` at all.
///
/// The control is per query: the equality is asserted wherever `LOW_EF` and
/// `HIGH_EF` answer differently, and at least one query must, across all the
/// `options` of a call. A single query can sit where even `LOW_EF`'s narrow
/// beam finds the exact answer (see `HARD_POINTS`), which would make the
/// control vacuous for that query.
fn assert_follows_the_configured_ef(
    collection: &velesdb_core::VectorCollection,
    filtered: bool,
    options: &[&str],
) {
    let with_ef = |ef: usize, option: &str| match option {
        "" => format!("ef_search={ef}"),
        _ => format!("ef_search={ef}, {option}"),
    };
    let mut control_held = false;
    for (qi, query) in hard_queries().iter().enumerate() {
        for option in options {
            let run = |with: &str| with_query_ids(collection, query, filtered, with);
            let configured = run(option);
            let low = run(&with_ef(LOW_EF, option));
            let high = run(&with_ef(HIGH_EF, option));
            if low != high {
                control_held = true;
                assert_eq!(
                    configured, low,
                    "a query with `{option}` and no ef_search must follow the configured \
                     ef_search, not a hard-coded Balanced (query {qi}, filtered: {filtered})"
                );
            }
        }
    }
    assert!(
        control_held,
        "CONTROL: ef must change the answer for at least one of {QUERIES} queries \
         (filtered: {filtered}, options: {options:?})"
    );
}

/// A `NEAR` query naming no quality of its own reaches the configured
/// `[search]` quality: with no `WITH` at all, and with a `WITH` that sets only
/// `rerank` (#2399), the latter on the plain vector path (`vector.rs`'s
/// `search_with_opts`) and on the metadata-filtered one (`vector_filter.rs`'s
/// `search_with_filter_and_opts`; the fixture's `cat` field has no secondary
/// index).
///
/// One test, one `HARD_POINTS` build: the cases read the same fixture, and a
/// build per case would add about as much CI time each.
#[test]
fn a_configured_ef_search_reaches_a_near_query_naming_no_quality() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    let collection = seeded_hard(&dir, config_with_ef(LOW_EF));
    assert_follows_the_configured_ef(&collection, false, &[""]);
    assert_follows_the_configured_ef(&collection, false, &["rerank=true", "rerank=false"]);
    assert_follows_the_configured_ef(&collection, true, &["rerank=true", "rerank=false"]);
}

/// One answer per query, for each batch / multi-query entry point.
struct BatchAnswers {
    parallel: Vec<Vec<u64>>,
    with_filters: Vec<Vec<u64>>,
    multi: Vec<Vec<u64>>,
}

/// Runs the three entry points, none of which takes a per-call quality.
///
/// `WIDE_K` and `MULTI_K` are not the `K` of the other tests: each entry point
/// overfetches by a `k`-dependent factor, and at `K` the final top-k agrees
/// for `LOW_EF` and `HIGH_EF` even where the wider candidate pools differ.
/// `Collection::overfetch_factor` is tiered (x20 up to 10, x10 up to 50, x5 up
/// to 100, x2 beyond), so a larger `k` does not always mean a wider window:
/// 101 gives 202 where 50 gives 500, which is why `multi_query_search` uses
/// `MULTI_K`. Both values were chosen by measurement, not derivation.
fn batch_answers(
    collection: &velesdb_core::VectorCollection,
    queries: &[Vec<f32>],
) -> BatchAnswers {
    let per_query = |answer: &dyn Fn(&[f32]) -> Vec<u64>| -> Vec<Vec<u64>> {
        queries.iter().map(|q| answer(q)).collect()
    };
    BatchAnswers {
        parallel: per_query(&|q| {
            let batch = collection.search_batch_parallel(&[q], K);
            ids(&batch.expect("test: batch_parallel")[0])
        }),
        with_filters: per_query(&|q| {
            let batch = collection.search_batch_with_filters(&[q], WIDE_K, &[None]);
            ids(&batch.expect("test: batch_with_filters")[0])
        }),
        multi: per_query(&|q| {
            let fusion = velesdb_core::fusion::FusionStrategy::Maximum;
            ids(&collection
                .multi_query_search(&[q], MULTI_K, fusion, None)
                .expect("test: multi_query_search"))
        }),
    }
}

fn some_query_disagrees(low: &[Vec<u64>], high: &[Vec<u64>]) -> bool {
    low.iter().zip(high).any(|(l, h)| l != h)
}

/// The configured `[search]` quality reaches the batch and multi-query entry
/// points (`search_batch_parallel`, `search_batch_with_filters`,
/// `multi_query_search`), which take no quality of their own.
///
/// One persisted index reopened under two configured defaults must not answer
/// identically for every query: if the config never reached the call, both
/// opens would run the same hard-coded `Balanced` and agree on all of them,
/// whatever the build's randomness (see `HARD_POINTS`).
#[test]
fn a_configured_ef_search_reaches_the_batch_and_multi_query_entry_points() {
    let dir = tempfile::TempDir::new().expect("test: tempdir");
    seeded_n(&dir, VelesConfig::default(), HARD_POINTS)
        .flush_full()
        .expect("test: persist the index both opens will read");
    let queries = hard_queries();

    let (db_low, low) = reopened(&dir, config_with_ef(LOW_EF));
    let low_answers = batch_answers(&low, &queries);
    drop(low);
    drop(db_low);
    let (_db_high, high) = reopened(&dir, config_with_ef(HIGH_EF));
    let high_answers = batch_answers(&high, &queries);

    assert!(
        some_query_disagrees(&low_answers.parallel, &high_answers.parallel),
        "search_batch_parallel must follow the configured ef_search, not a hard-coded Balanced"
    );
    assert!(
        some_query_disagrees(&low_answers.with_filters, &high_answers.with_filters),
        "search_batch_with_filters must follow the configured ef_search, not a hard-coded Balanced"
    );
    assert!(
        some_query_disagrees(&low_answers.multi, &high_answers.multi),
        "multi_query_search must follow the configured ef_search, not a hard-coded Balanced"
    );
}
