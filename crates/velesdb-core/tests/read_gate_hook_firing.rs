//! Read-path control-plane gate — read-hook firing and context.
//!
//! Feature: core-control-plane-boundary, Property 2
//!
//! **Property 2: Read hook fires before results with correct context** —
//! for every gated read path, `on_query_request` is invoked exactly once,
//! before any result is produced, with a [`QueryAccessContext`] carrying the
//! correct collection name and the matching [`QueryOperationKind`].
//!
//! **Validates: Requirements 1.1, 1.2, 1.7**
//!
//! A single spy observer records each `on_query_request` `(collection,
//! operation)` pair. A shared monotonic sequence counter is stamped in BOTH
//! `on_query_request` and `on_query` (the latter fires only *after* results
//! are produced, per Task 5.1); the request stamp being strictly less than the
//! query stamp proves the read hook fired before results. The observer always
//! returns [`AccessDecision::Allow`], so the underlying query genuinely
//! executes and the ordering is exercised end-to-end. Each iteration drives a
//! query with a KNOWN operation kind — a relational `SELECT` (`Select`), a
//! `NEAR` vector search (`VectorSearch`), and a bare `MATCH` graph traversal
//! (`GraphTraversal`) — so the derived context can be checked exactly.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use proptest::prelude::*;
use serde_json::{json, Value};
use tempfile::TempDir;
use velesdb_core::{
    velesql::Parser, AccessDecision, Database, DatabaseObserver, DistanceMetric, GraphEdge, Point,
    QueryAccessContext, QueryOperationKind,
};

/// Sentinel for "no stamp recorded yet" — distinct from any real sequence
/// value so the assertions can tell a missing hook from an early one.
const NOT_STAMPED: usize = usize::MAX;

/// Spy observer that records every read-hook invocation and stamps a shared
/// monotonic sequence counter in both the request hook and the completion
/// telemetry hook, so the test can prove the request fires before results.
struct SpyObserver {
    /// Monotonic tick handed out to each stamped event.
    seq: AtomicUsize,
    /// Sequence value stamped when `on_query_request` fired.
    request_stamp: AtomicUsize,
    /// Sequence value stamped when `on_query` (post-results) fired.
    query_stamp: AtomicUsize,
    /// Each `(collection, operation)` observed on the read hook, in order.
    requests: Mutex<Vec<(String, QueryOperationKind)>>,
}

impl SpyObserver {
    fn new() -> Self {
        Self {
            seq: AtomicUsize::new(0),
            request_stamp: AtomicUsize::new(NOT_STAMPED),
            query_stamp: AtomicUsize::new(NOT_STAMPED),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<(String, QueryOperationKind)> {
        self.requests.lock().clone()
    }

    fn request_stamp(&self) -> usize {
        self.request_stamp.load(Ordering::SeqCst)
    }

    fn query_stamp(&self) -> usize {
        self.query_stamp.load(Ordering::SeqCst)
    }
}

impl DatabaseObserver for SpyObserver {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let stamp = self.seq.fetch_add(1, Ordering::SeqCst);
        self.request_stamp.store(stamp, Ordering::SeqCst);
        self.requests
            .lock()
            .push((ctx.collection.to_string(), ctx.operation));
        Ok(AccessDecision::Allow)
    }

    fn on_query(&self, _collection: &str, _duration_us: u64) {
        let stamp = self.seq.fetch_add(1, Ordering::SeqCst);
        self.query_stamp.store(stamp, Ordering::SeqCst);
    }
}

/// Creates the `items` vector collection with rows that both the relational
/// and vector-search queries can return.
fn setup_items(db: &Database) {
    db.create_vector_collection("items", 4, DistanceMetric::Cosine)
        .expect("test: create items collection");
    let collection = db
        .get_vector_collection("items")
        .expect("test: get items collection");
    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 100})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "science", "price": 25})),
            ),
            Point::new(
                3,
                vec![0.8, 0.2, 0.0, 0.0],
                Some(json!({"_labels": ["Item"], "category": "tech", "price": 50})),
            ),
        ])
        .expect("test: upsert items");
}

/// Creates the `social` graph (a `Start`-anchored KNOWS chain) that the bare
/// `MATCH` traversal walks. Mirrors the fixed-topology pattern used by the
/// graph BDD suite: nodes carry `_labels`, edges are added via `add_edge`.
fn setup_social(db: &Database) {
    db.create_vector_collection("social", 4, DistanceMetric::Cosine)
        .expect("test: create social collection");
    let vc = db
        .get_vector_collection("social")
        .expect("test: get social collection");
    vc.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"_labels": ["Start"], "name": "A"})),
        ),
        Point::new(2, vec![0.0, 1.0, 0.0, 0.0], Some(json!({"name": "B"}))),
        Point::new(3, vec![0.0, 0.0, 1.0, 0.0], Some(json!({"name": "C"}))),
    ])
    .expect("test: upsert social nodes");
    for (edge_id, src, dst) in [(100u64, 1u64, 2u64), (101, 2, 3)] {
        let edge = GraphEdge::new(edge_id, src, dst, "KNOWS").expect("test: create social edge");
        vc.add_edge(edge).expect("test: add social edge");
    }
}

/// Builds a read query with a KNOWN operation kind from the proptest inputs,
/// returning the SQL, its execution params, and the `(collection, kind)` the
/// derived [`QueryAccessContext`] must carry.
///
/// * `Select` — plain relational scan over `items`.
/// * `VectorSearch` — `NEAR` similarity search over `items`.
/// * `GraphTraversal` — bare `MATCH` routed via the `_collection` param; a bare
///   MATCH has an empty AST `from`, so the context collection is `""`.
fn build_case(
    shape: u8,
    threshold: i64,
    limit: u32,
) -> (
    String,
    HashMap<String, Value>,
    &'static str,
    QueryOperationKind,
) {
    match shape % 3 {
        0 => (
            format!("SELECT * FROM items WHERE price > {threshold} LIMIT {limit}"),
            HashMap::new(),
            "items",
            QueryOperationKind::Select,
        ),
        1 => (
            format!("SELECT * FROM items WHERE vector NEAR [1.0, 0.0, 0.0, 0.0] LIMIT {limit}"),
            HashMap::new(),
            "items",
            QueryOperationKind::VectorSearch,
        ),
        _ => {
            let mut params = HashMap::new();
            params.insert("_collection".to_string(), json!("social"));
            (
                format!("MATCH (a:Start)-[:KNOWS*1..1]->(c) RETURN c LIMIT {limit}"),
                params,
                "",
                QueryOperationKind::GraphTraversal,
            )
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Property 2: for every gated read path the read hook fires exactly once,
    /// before results, carrying the correct collection and operation kind.
    #[test]
    fn read_hook_fires_once_before_results_with_correct_context(
        shape in any::<u8>(),
        threshold in 0i64..=200,
        limit in 1u32..=10,
    ) {
        let (sql, params, expected_collection, expected_kind) =
            build_case(shape, threshold, limit);

        let spy = Arc::new(SpyObserver::new());
        let dir = TempDir::new().expect("test: tempdir");
        let db = Database::open_with_observer(
            dir.path(),
            spy.clone() as Arc<dyn DatabaseObserver>,
        )
        .expect("test: open spy-observer db");
        setup_items(&db);
        setup_social(&db);

        let query = Parser::parse(&sql).expect("test: parse read query");
        // Allow path: the query genuinely executes so the ordering vs results
        // is exercised end-to-end.
        db.execute_query(&query, &params)
            .expect("test: allow path must execute");

        // The read hook fired exactly once for the top-level query.
        let requests = spy.requests();
        prop_assert_eq!(
            requests.len(),
            1_usize,
            "read hook must fire exactly once, got {} for sql {}",
            requests.len(),
            sql
        );

        // ...with the correct collection and operation kind.
        let (captured_collection, captured_kind) = &requests[0];
        prop_assert_eq!(
            captured_collection.as_str(),
            expected_collection,
            "captured collection must match queried collection for sql {}",
            sql
        );
        prop_assert_eq!(
            *captured_kind,
            expected_kind,
            "captured operation kind must match expected for sql {}",
            sql
        );

        // The request stamp precedes the results/completion stamp: the hook
        // fired BEFORE any result was produced.
        let request_stamp = spy.request_stamp();
        let query_stamp = spy.query_stamp();
        prop_assert!(
            request_stamp != NOT_STAMPED,
            "read hook must have stamped the request for sql {}",
            sql
        );
        prop_assert!(
            query_stamp != NOT_STAMPED,
            "query completion must have stamped after results for sql {}",
            sql
        );
        prop_assert!(
            request_stamp < query_stamp,
            "read hook must fire before results: request stamp {} vs query stamp {}",
            request_stamp,
            query_stamp
        );
    }
}
