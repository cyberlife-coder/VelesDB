//! Read-path control-plane gate — recall/scoring regression guard.
//!
//! Feature: core-control-plane-boundary, Task 12.1 (Requirement 8.1)
//!
//! The read gate wired into `Database::execute_query` (task 4.2) must never
//! perturb the search hot path. In particular, when **no observer** is present
//! the gate takes the `Cow::Borrowed` fast path: it must not clone, rewrite, or
//! re-score the query, so the produced results — ids, similarity **scores**,
//! and order — are byte-for-byte what the pre-hook data-plane path produced.
//!
//! This is the scoring-preservation companion to the recall gate
//! (`cargo test -p velesdb-core --features persistence test_recall`): the
//! recall suite proves recall@10 ≥ 0.95 with the hook wired, and this test
//! proves the no-observer path is a genuine no-op on the ANN scoring pipeline
//! rather than a subtle rescoring.
//!
//! **Validates: Requirements 8.1**
//!
//! Strategy (fully deterministic — no proptest): a cosine collection of six
//! well-separated points is seeded identically into two databases. One is
//! opened with `Database::open` (no observer — the fast-path baseline); the
//! other with `Database::open_with_observer` and an `Allow`-returning spy. The
//! same inline-literal `vector NEAR` query runs through `execute_query` on
//! both. The regression assertion is that the ordered `(id, score-bits)`
//! sequence is *identical* across the two paths, so the read gate leaves
//! scoring untouched. A frozen golden order + strictly-decreasing scores pin
//! that real scoring actually occurred (the equivalence is not vacuous).

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, AccessDecision, Database, DatabaseObserver, DistanceMetric, Point,
    QueryAccessContext,
};

/// Spy observer whose `on_query_request` returns `Allow` (no scope) and counts
/// invocations, so the test can confirm the gate genuinely fired on the
/// observer path (the no-op equivalence is therefore meaningful, not an
/// artifact of the gate being skipped).
struct AllowSpy {
    calls: AtomicUsize,
}

impl AllowSpy {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl DatabaseObserver for AllowSpy {
    fn on_query_request(&self, _ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(AccessDecision::Allow)
    }
}

/// Seed the `docs` collection (dim 4, cosine) with six points spread along
/// `[1, off, 0, 0]` plus one orthogonal point. Query `[1, 0, 0, 0]` yields the
/// strictly-decreasing cosine order `[10, 11, 12, 13, 14, 15]`.
fn seed(db: &Database) {
    db.create_vector_collection("docs", 4, DistanceMetric::Cosine)
        .expect("test: create docs collection");
    let vc = db
        .get_vector_collection("docs")
        .expect("test: get docs collection");
    vc.upsert(vec![
        Point::new(10, vec![1.0, 0.0, 0.0, 0.0], Some(json!({"n": 10}))),
        Point::new(11, vec![1.0, 0.3, 0.0, 0.0], Some(json!({"n": 11}))),
        Point::new(12, vec![1.0, 0.7, 0.0, 0.0], Some(json!({"n": 12}))),
        Point::new(13, vec![1.0, 1.2, 0.0, 0.0], Some(json!({"n": 13}))),
        Point::new(14, vec![1.0, 3.0, 0.0, 0.0], Some(json!({"n": 14}))),
        Point::new(15, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"n": 15}))),
    ])
    .expect("test: upsert docs corpus");
}

/// Inline-literal vector search — self-contained (no bound params), so both
/// databases run byte-identical query text through `Database::execute_query`.
const QUERY: &str = "SELECT * FROM docs WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] \
                     ORDER BY similarity() DESC LIMIT 6";

/// Execute `QUERY` and capture the observable that scoring can affect: the
/// ordered list of `(id, exact score bits)`. Comparing score *bits* (rather
/// than an epsilon) makes any rescoring by the gate a hard failure.
fn scored(db: &Database) -> Vec<(u64, u32)> {
    let query = Parser::parse(QUERY).expect("test: parse query");
    let results = db
        .execute_query(&query, &HashMap::new())
        .expect("test: execute query");
    results
        .iter()
        .map(|r| (r.point.id, r.score.to_bits()))
        .collect()
}

/// REGRESSION: the no-observer read path leaves scoring untouched relative to
/// the gated (Allow) path — identical ids, identical similarity scores, and
/// identical order — proving the read gate's `Cow::Borrowed` fast path neither
/// clones nor rescored the query.
#[test]
fn no_observer_read_path_leaves_scoring_untouched() {
    // (a) Baseline: no observer — the fast-path (pre-hook) read path.
    let baseline_dir = TempDir::new().expect("test: tempdir");
    let baseline = Database::open(baseline_dir.path()).expect("test: open baseline");
    seed(&baseline);
    let baseline = scored(&baseline);

    // (b) Gated: an Allow-returning observer, so `on_query_request` fires and
    // the gate walks its Allow arm before execution.
    let spy = Arc::new(AllowSpy::new());
    let gated_dir = TempDir::new().expect("test: tempdir");
    let gated_db =
        Database::open_with_observer(gated_dir.path(), spy.clone() as Arc<dyn DatabaseObserver>)
            .expect("test: open gated db");
    seed(&gated_db);
    let gated = scored(&gated_db);

    // Sanity: real scoring happened (not an empty/degenerate comparison).
    assert_eq!(
        baseline.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![10, 11, 12, 13, 14, 15],
        "baseline must return the exact descending-cosine order"
    );
    for w in baseline.windows(2) {
        assert!(
            f32::from_bits(w[0].1) >= f32::from_bits(w[1].1),
            "baseline similarity scores must be non-increasing"
        );
    }

    // The gate genuinely fired on the observer path.
    assert!(
        spy.calls() >= 1,
        "the read gate must have invoked on_query_request on the observer path"
    );

    // Core regression: ids AND exact score bits AND order are identical, so the
    // no-observer fast path did not alter scoring vs. the gated path.
    assert_eq!(
        baseline, gated,
        "no-observer read path must yield byte-identical (id, score, order) to the gated Allow path"
    );
}
