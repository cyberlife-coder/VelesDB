//! Fetch depths, cutoff ties and result-cardinality guards.

use super::super::{fetch_past_ties, FetchWindow};
use super::query_params;
use crate::collection::search::query::QuerySearchOptions;
use crate::velesql::Parser;
use serde_json::json;

#[cfg(feature = "gpu")]
#[test]
fn eligible_device_reranking_keeps_original_keys_off_the_native_prefix() {
    use crate::{collection::Collection, DistanceMetric, Point};
    use std::collections::HashMap;

    const DIM: usize = 4096;
    const COUNT: u16 = 65;
    assert!(crate::gpu::GpuAccelerator::should_rerank_gpu(
        usize::from(COUNT),
        DIM
    ));
    for metric in [DistanceMetric::Euclidean, DistanceMetric::DotProduct] {
        let dir = tempfile::tempdir().expect("database");
        let col = Collection::create(dir.path().to_path_buf(), DIM, metric).expect("collection");
        col.upsert(
            (0..COUNT)
                .map(|i| Point::without_payload(u64::from(i), vec![f32::from(i); DIM]))
                .collect::<Vec<_>>(),
        )
        .expect("native candidates");
        let params = HashMap::from([("q".to_owned(), json!(vec![1.0; DIM]))]);
        for (key, hint, expected) in [
            (
                "similarity(vector, $q)",
                "rerank=true",
                FetchWindow::OrderedScan { pq_scores: false },
            ),
            ("similarity()", "rerank=true", FetchWindow::Bounded(3)),
            (
                "similarity(vector, $q)",
                "mode='perfect'",
                FetchWindow::Bounded(3),
            ),
        ] {
            let query = Parser::parse(&format!(
                "SELECT * FROM rows WHERE vector NEAR $q ORDER BY {key} DESC LIMIT 3 WITH ({hint})"
            ))
            .expect("select");
            let stmt = &query.select;
            let mut parts =
                Collection::extract_query_components(stmt, &params).expect("components");
            let mut opts =
                QuerySearchOptions::from_with_clause(stmt.with_clause.as_ref()).expect("options");
            assert_eq!(
                col.select_fetch_window(stmt, &mut parts, &params, 3, &mut opts),
                expected,
                "{metric:?}, {key}, {hint}"
            );
        }
    }
}

/// Interleave a live PQ enable between planning a bounded explicit key and
/// fetching it, using public training and ingestion on that same collection.
#[test]
fn a_bounded_f32_order_keeps_its_scorer_across_live_quantizer_training() {
    use crate::{Database, DistanceMetric, Point};
    use std::collections::HashMap;

    let dir = tempfile::tempdir().expect("database directory");
    let db = Database::open(dir.path()).expect("database");
    db.create_vector_collection("rows", 1, DistanceMetric::Euclidean)
        .expect("collection");
    let c = db.get_vector_collection("rows").expect("rows");
    c.upsert(vec![
        Point::without_payload(1, vec![0.0]),
        Point::without_payload(2, vec![10.0]),
    ])
    .expect("centroids");
    let sql = Parser::parse(
        "SELECT * FROM rows WHERE vector NEAR $q ORDER BY similarity(vector, $q) DESC LIMIT 1",
    )
    .expect("select");
    let params = HashMap::from([("q".to_owned(), json!([4.9]))]);
    let mut parts = crate::collection::Collection::extract_query_components(&sql.select, &params)
        .expect("extract");
    let mut opts = QuerySearchOptions::default();
    assert_eq!(
        c.inner
            .select_fetch_window(&sql.select, &mut parts, &params, 1, &mut opts),
        FetchWindow::Bounded(1)
    );

    let train =
        Parser::parse("TRAIN QUANTIZER ON rows WITH (m=1, k=2, oversampling=4)").expect("train");
    db.execute_query(&train, &params).expect("enable PQ");
    c.upsert(vec![Point::without_payload(3, vec![5.1])])
        .expect("point in far code");
    assert_eq!(c.search(&[4.9], 1).expect("native ADC")[0].point.id, 1);
    assert_eq!(
        c.inner
            .search_with_opts(&[4.9], 1, &opts)
            .expect("bounded f32")[0]
            .point
            .id,
        3
    );
}

#[test]
fn similarity_score_order_reads_through_both_signs_of_a_cutoff_tie() {
    use crate::point::{Point, SearchResult};
    let query = Parser::parse("SELECT * FROM docs ORDER BY similarity() DESC LIMIT 1")
        .expect("test: statement");
    for (left, right) in [(-0.0, 0.0), (f32::NAN, -f32::NAN)] {
        let mut reads = Vec::new();
        let rows = fetch_past_ties(&query.select, 1, |k| {
            reads.push(k);
            Ok([(3, left), (2, right), (1, left)]
                .into_iter()
                .take(k)
                .map(|(id, score)| SearchResult::new(Point::new(id, Vec::new(), None), score))
                .collect())
        })
        .expect("test: fetch");
        assert_eq!(reads, [2, 4]);
        assert!(rows.iter().any(|r| r.point.id == 1));
    }
}

/// The cardinality guard-rail counts the rows a statement returns, not the
/// rows its window was selected from: an ORDER BY that ranks every row of a
/// collection larger than the guard still returns its window.
#[test]
fn the_cardinality_guard_counts_the_rows_returned() {
    use crate::guardrails::{GuardRails, QueryLimits};
    use crate::point::Point;

    let (_dir, mut col) = crate::test_fixtures::fixtures::setup_collection(2);
    #[allow(clippy::cast_precision_loss)]
    let points: Vec<Point> = (1..=400u64)
        .map(|id| {
            let payload = json!({"cat": id % 2});
            Point::new(id, vec![1.0, id as f32 / 100.0], Some(payload))
        })
        .collect();
    col.upsert(points).expect("test: upsert");
    col.runtime.guard_rails = std::sync::Arc::new(GuardRails::with_limits(QueryLimits {
        max_cardinality: 50,
        ..QueryLimits::default()
    }));
    let run = |sql: &str| {
        let query = Parser::parse(sql).unwrap_or_else(|e| panic!("test: parse {sql}: {e}"));
        col.execute_query(&query, &query_params())
    };
    for sql in [
        "SELECT * FROM c WHERE vector NEAR $q ORDER BY similarity() * 2 DESC LIMIT 5",
        "SELECT * FROM c ORDER BY similarity(vector, $q) DESC LIMIT 5",
        "SELECT * FROM c WHERE cat = 1 ORDER BY similarity(vector, $q) DESC LIMIT 5",
        "SELECT * FROM c WHERE vector NEAR $q ORDER BY similarity() ASC LIMIT 5",
        "SELECT * FROM c WHERE similarity(vector, $q) > 0.0 \
         ORDER BY similarity(vector, $q) ASC LIMIT 5",
        "SELECT * FROM c ORDER BY cat DESC LIMIT 5",
    ] {
        let rows = run(sql).unwrap_or_else(|e| panic!("test: {sql}: {e}"));
        assert_eq!(rows.len(), 5, "{sql}");
    }
    // The guard still refuses a statement returning more rows than it allows.
    let refused = run("SELECT * FROM c ORDER BY cat DESC LIMIT 60");
    assert!(
        matches!(refused, Err(crate::error::Error::GuardRail { .. })),
        "{refused:?}"
    );
}

/// The window each statement's fetch reads, under a metric where higher is
/// more similar and under two where lower is: the bounded `LIMIT + OFFSET`
/// only for no `ORDER BY`, or a single `similarity()` key read `DESC` that
/// the fetch ranks by — a `NEAR` or an index-bounded threshold on that
/// vector, a sparse score only where higher is more similar; the ordered
/// scan for any other order over rows a scan scores; every row up to the
/// ceiling for a fusion, an own score read otherwise, DISTINCT, a vector
/// GROUP BY and a window function.
#[test]
fn only_an_order_the_fetch_ranks_by_keeps_the_bounded_fetch() {
    use crate::collection::Collection;
    use crate::distance::DistanceMetric;

    for metric in [
        DistanceMetric::Cosine,
        DistanceMetric::Euclidean,
        DistanceMetric::Hamming,
    ] {
        let dir = tempfile::tempdir().expect("test: tempdir");
        let col = Collection::create(dir.path().to_path_buf(), 2, metric).expect("test: create");
        let cases = window_cases(metric);
        for (sql, want) in cases {
            let query = Parser::parse(&sql).unwrap_or_else(|e| panic!("test: parse {sql}: {e}"));
            let stmt = &query.select;
            let (_, fetch_limit) = Collection::compute_fetch_limit(stmt);
            let mut extracted =
                Collection::extract_query_components(stmt, &query_params()).expect("test: extract");
            let got = col.select_fetch_window(
                stmt,
                &mut extracted,
                &query_params(),
                fetch_limit,
                &mut QuerySearchOptions::default(),
            );
            assert_eq!(got, want, "{metric:?}: {sql}");
        }
    }
}

const NEAR: &str = "SELECT * FROM c WHERE vector NEAR $q";
const THRESHOLD: &str = "SELECT * FROM c WHERE similarity(vector, $q) > 0.1";
const FILTERED: &str = "SELECT * FROM c WHERE cat = 'a'";
const SPARSE: &str = "SELECT * FROM c WHERE vector SPARSE_NEAR $s";
const HYBRID: &str = "SELECT * FROM c WHERE vector NEAR $q AND content MATCH 'banana'";
const FUSED: &str = "SELECT * FROM c WHERE vector NEAR_FUSED [$q, $far]";

/// The statements `only_an_order_the_fetch_ranks_by_keeps_the_bounded_fetch`
/// reads, each with its window under a metric where higher is more similar
/// (`higher`) or not.
fn window_cases(metric: crate::DistanceMetric) -> Vec<(String, FetchWindow)> {
    let mut cases = bounded_cases(metric);
    cases.extend(ordered_scan_cases());
    cases.extend(exhaustive_cases());
    cases
}

/// The windows a fetch stops at: its `LIMIT + OFFSET`.
fn bounded_cases(metric: crate::DistanceMetric) -> Vec<(String, FetchWindow)> {
    use FetchWindow::{Bounded, Exhaustive};

    let either = |bounded| if bounded { Bounded(3) } else { Exhaustive };
    let original = match metric {
        crate::DistanceMetric::Cosine | crate::DistanceMetric::Jaccard => {
            FetchWindow::OrderedScan { pq_scores: false }
        }
        _ => Bounded(3),
    };
    vec![
        (format!("{NEAR} LIMIT 3"), Bounded(3)),
        (
            format!("{NEAR} ORDER BY similarity() DESC LIMIT 3 OFFSET 2"),
            Bounded(5),
        ),
        (
            format!("{NEAR} ORDER BY similarity(vector, $q) DESC LIMIT 3"),
            original,
        ),
        (
            format!("{NEAR} ORDER BY similarity() DESC, cat ASC LIMIT 3"),
            FetchWindow::OrderedScan { pq_scores: false },
        ),
        (
            format!("{THRESHOLD} ORDER BY similarity(vector, $q) DESC LIMIT 3"),
            original,
        ),
        (
            "SELECT * FROM c WHERE content MATCH 'banana' AND similarity(vector, $q) > 0.1 \
             ORDER BY similarity(vector, $q) DESC LIMIT 3"
                .to_string(),
            original,
        ),
        (format!("{FILTERED} LIMIT 3"), Bounded(3)),
        (format!("{SPARSE} LIMIT 3"), Bounded(3)),
        (
            format!("{SPARSE} ORDER BY similarity() DESC LIMIT 3"),
            either(metric.higher_is_better()),
        ),
    ]
}

/// The orders the ordered scan reads every admitted row for.
fn ordered_scan_cases() -> Vec<(String, FetchWindow)> {
    let scan = FetchWindow::OrderedScan { pq_scores: false };

    vec![
        (format!("{NEAR} ORDER BY similarity() ASC LIMIT 3"), scan),
        (
            format!("{NEAR} ORDER BY similarity(vector, $far) DESC LIMIT 3"),
            scan,
        ),
        (
            format!("{NEAR} ORDER BY similarity(emb, $q) DESC LIMIT 3"),
            scan,
        ),
        (
            format!("{NEAR} ORDER BY 1 - similarity() DESC LIMIT 3"),
            scan,
        ),
        (format!("{NEAR} ORDER BY cat ASC LIMIT 3"), scan),
        (
            format!("{THRESHOLD} ORDER BY similarity(vector, $q) ASC LIMIT 3"),
            scan,
        ),
        (
            format!("{FILTERED} ORDER BY similarity(vector, $q) DESC LIMIT 3"),
            scan,
        ),
        (
            format!("{FILTERED} ORDER BY similarity() DESC LIMIT 3"),
            scan,
        ),
        (
            "SELECT * FROM c ORDER BY similarity(vector, $q) DESC LIMIT 3".to_string(),
            scan,
        ),
    ]
}

/// The reads up to the ceiling.
fn exhaustive_cases() -> Vec<(String, FetchWindow)> {
    use FetchWindow::Exhaustive;

    vec![
        (
            "SELECT * FROM c WHERE content MATCH 'banana' ORDER BY cat LIMIT 3".to_string(),
            Exhaustive,
        ),
        (
            format!("{SPARSE} ORDER BY similarity() ASC LIMIT 3"),
            Exhaustive,
        ),
        (
            format!("{HYBRID} ORDER BY similarity() DESC LIMIT 3"),
            Exhaustive,
        ),
        (
            format!("{FUSED} ORDER BY similarity() DESC LIMIT 3"),
            Exhaustive,
        ),
        (
            "SELECT DISTINCT cat FROM c WHERE vector NEAR $q LIMIT 3".to_string(),
            Exhaustive,
        ),
        (
            "SELECT cat FROM c WHERE vector NEAR $q GROUP BY cat LIMIT 3".to_string(),
            Exhaustive,
        ),
        (
            "SELECT id, ROW_NUMBER() OVER (ORDER BY cat) AS n FROM c \
             ORDER BY cat LIMIT 3"
                .to_string(),
            Exhaustive,
        ),
    ]
}

#[test]
fn only_a_bare_or_grouped_near_is_an_unfiltered_native_prefix() {
    use crate::collection::Collection;
    let params = std::collections::HashMap::from([("q".to_owned(), json!([1.0, 0.0]))]);
    for (predicate, pure) in [
        ("vector NEAR $q", true),
        ("((vector NEAR $q))", true),
        ("vector NEAR $q AND id > 7", false),
        ("vector NEAR $q AND similarity(vector, $q) > 0.5", false),
        ("vector NEAR $q AND (id > 7 OR id < 2)", false),
        ("vector NEAR $q AND MATCH (d)-[:CITES]->(other)", false),
        (
            "vector NEAR $q AND (MATCH (d)-[:CITES]->(other) OR id = 3)",
            false,
        ),
    ] {
        let sql =
            Parser::parse(&format!("SELECT * FROM docs AS d WHERE {predicate}")).expect("query");
        let extracted = Collection::extract_query_components(&sql.select, &params).expect("NEAR");
        assert_eq!(extracted.is_unfiltered_near(), pure, "{predicate}");
    }
}
