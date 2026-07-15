//! WO-D2: observability of the `SCAN_CAP` truncation in
//! `scan_and_score_by_vector` (GraphFirst full-scan + exact rescoring).
//!
//! The cap itself (100 000, #901 pathological-query guard) is intentional and
//! unchanged; these tests prove that when the cap actually bites (the scan
//! stopped while candidate ids were still unvisited) a single structured
//! `tracing::warn!` is emitted per query, and that the returned results stay
//! correct and ordered. The cap is exercised at a small test value through
//! `scan_and_score_by_vector_capped` instead of inserting 100k points.

#![cfg(all(test, feature = "persistence"))]

use crate::point::Point;
use crate::test_fixtures::fixtures::{make_point_with_payload, setup_collection};
use std::sync::{Arc, Mutex};

// ============================================================================
// Minimal warn-capturing tracing subscriber (no extra dev-dependency).
// ============================================================================

/// Captures WARN-and-worse events as flat `field=value` strings.
#[derive(Default)]
struct WarnSink {
    events: Mutex<Vec<String>>,
}

struct FlattenVisitor<'a>(&'a mut String);

impl tracing::field::Visit for FlattenVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

struct WarnCapture {
    sink: Arc<WarnSink>,
}

impl tracing::Subscriber for WarnCapture {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        // In `tracing`, more severe levels compare LESS (ERROR < WARN < INFO).
        *metadata.level() <= tracing::Level::WARN
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = String::new();
        event.record(&mut FlattenVisitor(&mut line));
        self.sink
            .events
            .lock()
            .expect("test: warn sink lock")
            .push(line);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Runs `f` with a warn-capturing subscriber installed (thread-local) and
/// returns the captured WARN+ events.
fn capture_warns<T>(f: impl FnOnce() -> T) -> (T, Vec<String>) {
    let sink = Arc::new(WarnSink::default());
    let subscriber = WarnCapture { sink: sink.clone() };
    let out = tracing::subscriber::with_default(subscriber, f);
    let events = sink.events.lock().expect("test: warn sink lock").clone();
    (out, events)
}

// ============================================================================
// Fixture
// ============================================================================

/// 20 points, all matching `category = "tech"`. The second vector component
/// DEcreases with the id, so cosine similarity to `[1,0,0,0]` strictly
/// INcreases with the id: the globally best matches are the LAST inserted
/// points. A truncating scan that only visits an id-ordered prefix therefore
/// returns visibly different (worse) top-k than an exhaustive scan —
/// making silent truncation detectable from the results themselves.
fn setup_truncatable_fixture() -> (tempfile::TempDir, crate::collection::Collection) {
    let (dir, col) = setup_collection(4);
    let points: Vec<Point> = (1..=20u64)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let v = vec![1.0, (21 - i) as f32 * 0.05, 0.0, 0.0];
            make_point_with_payload(i, v, serde_json::json!({"category": "tech"}))
        })
        .collect();
    col.upsert(points).expect("test: upsert");
    (dir, col)
}

fn tech_filter() -> crate::filter::Filter {
    crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".into(),
        value: serde_json::json!("tech"),
    })
}

// ============================================================================
// Tests
// ============================================================================

/// GIVEN 20 matching points and a scan cap of 8
/// WHEN scan_and_score_by_vector runs with the cap biting
/// THEN exactly ONE structured warn is emitted for the whole query,
///      carrying the collection name, the cap, and the unscanned estimate.
#[test]
fn test_scan_cap_truncation_emits_single_structured_warn() {
    let (_dir, col) = setup_truncatable_fixture();
    let filter = tech_filter();
    let query = [1.0, 0.0, 0.0, 0.0];

    let (results, warns) =
        capture_warns(|| col.scan_and_score_by_vector_capped(&filter, &query, 5, 8));

    assert_eq!(results.len(), 5, "limit must still be respected");
    let cap_warns: Vec<&String> = warns.iter().filter(|w| w.contains("scan_cap=8")).collect();
    assert_eq!(
        cap_warns.len(),
        1,
        "exactly one truncation warn per query (not per candidate), got: {warns:?}"
    );
    let warn = cap_warns[0];
    assert!(
        warn.contains("collection="),
        "warn must identify the collection: {warn}"
    );
    assert!(
        warn.contains("matches_scored=8"),
        "warn must report how many matches were scored: {warn}"
    );
    assert!(
        warn.contains("unscanned_points=12"),
        "warn must estimate the unvisited remainder (20 total - 8 visited): {warn}"
    );
}

/// GIVEN the cap bites (only the first 8 of 20 matches are scored)
/// THEN the results are exactly the correctly-ordered top-k of the points the
///      scan actually visited — truncated, but never wrong or mis-ordered.
#[test]
fn test_scan_cap_truncated_results_stay_correct_and_ordered() {
    let (_dir, col) = setup_truncatable_fixture();
    let filter = tech_filter();
    let query = [1.0f32, 0.0, 0.0, 0.0];
    let (limit, cap) = (5usize, 8usize);

    let (results, _warns) =
        capture_warns(|| col.scan_and_score_by_vector_capped(&filter, &query, limit, cap));

    // Ground truth: the exact same first-`cap` matches the scan visits,
    // rescored by exact cosine and sorted best-first.
    let visited = col.execute_scan_query(&filter, cap, None);
    assert_eq!(visited.len(), cap, "fixture must saturate the cap");
    let mut expected: Vec<(u64, f32)> = visited
        .iter()
        .map(|r| {
            let score = crate::distance::DistanceMetric::Cosine
                .calculate(&r.point.vector, &query)
                .clamp(-1.0, 1.0);
            (r.point.id, score)
        })
        .collect();
    expected.sort_by(|a, b| b.1.total_cmp(&a.1));
    expected.truncate(limit);

    let got: Vec<(u64, f32)> = results.iter().map(|r| (r.point.id, r.score)).collect();
    assert_eq!(
        got.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        expected.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "top-k must be the correctly ordered best of the scanned prefix"
    );
    for (g, e) in got.iter().zip(expected.iter()) {
        assert!(
            (g.1 - e.1).abs() < f32::EPSILON,
            "scores must be the exact rescored similarity: got {g:?}, want {e:?}"
        );
    }
    for w in results.windows(2) {
        assert!(
            w[0].score >= w[1].score,
            "descending similarity order even under truncation"
        );
    }
}

/// GIVEN fewer matches than the cap (scan exhausts the collection)
/// THEN no truncation warn is emitted — the guard only fires when it bites.
#[test]
fn test_scan_cap_not_hit_emits_no_warn() {
    let (_dir, col) = setup_truncatable_fixture();
    let filter = tech_filter();
    let query = [1.0f32, 0.0, 0.0, 0.0];

    // Cap (32) far above the 20 available matches: nothing is truncated.
    let (results, warns) =
        capture_warns(|| col.scan_and_score_by_vector_capped(&filter, &query, 5, 32));

    assert_eq!(results.len(), 5);
    // Global best matches: ids 20, 19, 18, 17, 16 (similarity increases with id).
    assert_eq!(
        results.iter().map(|r| r.point.id).collect::<Vec<_>>(),
        vec![20, 19, 18, 17, 16],
        "without truncation the global top-k is returned"
    );
    assert!(
        !warns.iter().any(|w| w.contains("scan_cap=")),
        "no truncation warn when the cap does not bite, got: {warns:?}"
    );
}

/// GIVEN a filter matching NOTHING alongside a small cap
/// THEN the exhausted (empty) scan emits no warn — an empty result set is
///      not a truncation.
#[test]
fn test_scan_cap_no_matches_emits_no_warn() {
    let (_dir, col) = setup_truncatable_fixture();
    let filter = crate::filter::Filter::new(crate::filter::Condition::Eq {
        field: "category".into(),
        value: serde_json::json!("absent"),
    });
    let query = [1.0f32, 0.0, 0.0, 0.0];

    let (results, warns) =
        capture_warns(|| col.scan_and_score_by_vector_capped(&filter, &query, 5, 8));

    assert!(results.is_empty());
    assert!(
        !warns.iter().any(|w| w.contains("scan_cap=")),
        "no warn on an exhausted empty scan, got: {warns:?}"
    );
}

/// The production wrapper keeps the documented 100_000 cap: on a small
/// collection it never truncates and returns the exhaustive global top-k.
#[test]
fn test_public_wrapper_unchanged_no_warn_on_small_collection() {
    let (_dir, col) = setup_truncatable_fixture();
    let filter = tech_filter();
    let query = [1.0f32, 0.0, 0.0, 0.0];

    let (results, warns) = capture_warns(|| col.scan_and_score_by_vector(&filter, &query, 3));

    assert_eq!(
        results.iter().map(|r| r.point.id).collect::<Vec<_>>(),
        vec![20, 19, 18],
        "public path returns the exhaustive top-k"
    );
    assert!(
        !warns.iter().any(|w| w.contains("scan_cap=")),
        "public path must not warn below the cap, got: {warns:?}"
    );
}

/// `execute_scan_query_tracked` primitive: reports the exact number of
/// candidate ids left unvisited when the scan stops at `limit`, and zero
/// when the scan exhausts the id set.
#[test]
fn test_execute_scan_query_tracked_reports_unscanned_ids() {
    let (_dir, col) = setup_truncatable_fixture();
    let filter = tech_filter();

    // Stops after 8 matches: ids 9..=20 (12 points) were never visited.
    let truncated = col.execute_scan_query_tracked(&filter, 8, None);
    assert_eq!(truncated.results.len(), 8);
    assert_eq!(
        truncated.unscanned_ids, 12,
        "12 of 20 ids must be reported unvisited"
    );

    // Limit above the match count: the scan exhausts every id.
    let exhaustive = col.execute_scan_query_tracked(&filter, 64, None);
    assert_eq!(exhaustive.results.len(), 20);
    assert_eq!(
        exhaustive.unscanned_ids, 0,
        "exhausted scan leaves no remainder"
    );
}
