//! Read-path control-plane gate — every read a `VelesQL` statement makes.
//!
//! Feature: core-control-plane-boundary, Property 4 extended to aggregates and
//! to every collection a statement reads.
//!
//! The read gate (`DatabaseObserver::on_query_request`) must decide every read
//! of collection data, and its decision must hold whatever shape the read
//! takes:
//!
//! * `Deny(err)` aborts the statement with exactly `err`;
//! * `AllowWithScope(F)` AND-composes `F` into the rows the statement reads
//!   from that collection, so a scope can only narrow, never widen;
//! * where a read has no channel to apply `F` (a graph traversal, a JOIN's
//!   secondary collection, a `MATCH` `@collection` enrichment, `SELECT
//!   EDGES`), the statement is refused rather than run unscoped.
//!
//! Scenarios are written Given / When / Then. The fixture holds two
//! collections: `c` (six rows: three in category `a`, two in `b`, one with no
//! category) and `d` (three rows over two tenants). `amount` and `score` are
//! chosen so every aggregate tells a scoped answer from an unscoped one.

#![cfg(feature = "persistence")]

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use parking_lot::RwLock;
use proptest::prelude::*;
use serde_json::{json, Value as Json};
use tempfile::TempDir;
use velesdb_core::velesql::{Condition, Parser};
use velesdb_core::{
    AccessDecision, AccessScope, Database, DatabaseObserver, DistanceMetric, Error, GraphEdge,
    Point, QueryAccessContext,
};

/// What the observer answers for one collection.
#[derive(Clone)]
enum Rule {
    Deny(String),
    Scope(String),
}

/// Observer answering per collection, from a table switched between steps,
/// and recording every collection it was asked about.
#[derive(Default)]
struct PolicyObserver {
    rules: RwLock<HashMap<String, Rule>>,
    asked: RwLock<Vec<String>>,
}

impl PolicyObserver {
    fn scope(&self, collection: &str, predicate: &str) {
        self.rules
            .write()
            .insert(collection.to_string(), Rule::Scope(predicate.to_string()));
    }

    fn deny(&self, collection: &str, message: &str) {
        self.rules
            .write()
            .insert(collection.to_string(), Rule::Deny(message.to_string()));
    }

    fn allow_all(&self) {
        self.rules.write().clear();
    }

    fn asked(&self) -> Vec<String> {
        self.asked.read().clone()
    }
}

/// Parses a WHERE predicate into the `VelesQL` condition a scope carries.
fn condition(predicate: &str) -> Condition {
    Parser::parse(&format!("SELECT * FROM c WHERE {predicate}"))
        .expect("test: parse scope predicate")
        .select
        .where_clause
        .expect("test: scope predicate has a WHERE clause")
}

impl DatabaseObserver for PolicyObserver {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        self.asked.write().push(ctx.collection.to_string());
        let rule = self.rules.read().get(ctx.collection).cloned();
        Ok(match rule {
            None => AccessDecision::Allow,
            Some(Rule::Deny(message)) => AccessDecision::Deny(Error::Query(message)),
            Some(Rule::Scope(predicate)) => {
                #[allow(clippy::field_reassign_with_default)] // `AccessScope` is non_exhaustive
                let mut scope = AccessScope::default();
                scope.filter = Some(condition(&predicate));
                AccessDecision::AllowWithScope(scope)
            }
        })
    }
}

/// Opens a database under `observer` holding the `c` and `d` collections.
fn setup() -> (TempDir, Database, Arc<PolicyObserver>) {
    let observer = Arc::new(PolicyObserver::default());
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), observer.clone()).expect("test: open db");
    db.create_vector_collection("c", 4, DistanceMetric::Cosine)
        .expect("test: create c");
    db.create_vector_collection("d", 4, DistanceMetric::Cosine)
        .expect("test: create d");
    let rows: [(u64, Option<&str>, i64); 6] = [
        (1, Some("a"), 10),
        (2, Some("a"), 20),
        (3, Some("a"), 30),
        (4, Some("b"), 100),
        (5, Some("b"), 200),
        (6, None, 1000),
    ];
    let points: Vec<Point> = rows
        .iter()
        .map(|(id, cat, amount)| {
            let payload = match cat {
                Some(cat) => json!({"_labels": ["Item"], "cat": cat, "amount": amount}),
                None => json!({"_labels": ["Item"], "amount": amount}),
            };
            Point::new(*id, vec![1.0, 0.0, 0.0, 0.0], Some(payload))
        })
        .collect();
    let c = db.get_vector_collection("c").expect("test: get c");
    c.upsert(points).expect("test: seed c");
    let d = db.get_vector_collection("d").expect("test: get d");
    d.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"tenant": "x", "score": 5})),
        ),
        Point::new(
            2,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"tenant": "x", "score": 7})),
        ),
        Point::new(
            3,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(json!({"tenant": "y", "score": 1000})),
        ),
    ])
    .expect("test: seed d");
    (dir, db, observer)
}

fn aggregate(db: &Database, sql: &str) -> velesdb_core::Result<Json> {
    let query = Parser::parse(sql).expect("test: parse aggregate");
    db.execute_aggregate(&query, &HashMap::new())
}

fn rows(db: &Database, sql: &str) -> velesdb_core::Result<BTreeSet<u64>> {
    let query = Parser::parse(sql).expect("test: parse query");
    let results = db.execute_query(&query, &HashMap::new())?;
    Ok(results.into_iter().map(|r| r.point.id).collect())
}

/// The `n` of a scalar `COUNT(*) AS n` result.
fn count(db: &Database, sql: &str) -> i64 {
    aggregate(db, sql).expect("test: aggregate runs")["n"]
        .as_i64()
        .expect("test: n is an integer")
}

/// The group keys a `GROUP BY cat` result returns.
fn group_keys(result: &Json) -> BTreeSet<String> {
    result
        .as_array()
        .expect("test: GROUP BY returns an array")
        .iter()
        .map(|group| group["cat"].to_string())
        .collect()
}

fn assert_denied_with(result: velesdb_core::Result<impl std::fmt::Debug>, message: &str) {
    match result {
        Ok(value) => panic!("a denied read must not return data, got {value:?}"),
        Err(err) => {
            let expected = Error::Query(message.to_string());
            assert_eq!(
                err.code(),
                expected.code(),
                "deny keeps the supplied variant"
            );
            assert_eq!(
                err.to_string(),
                expected.to_string(),
                "deny keeps the message"
            );
        }
    }
}

fn assert_refused_unscoped(result: velesdb_core::Result<impl std::fmt::Debug>) {
    match result {
        Ok(value) => panic!("a scope the read cannot apply must refuse it, got {value:?}"),
        Err(err) => assert!(
            matches!(err, Error::Config(_)),
            "an inapplicable scope refuses with VELES-009, got {err:?}"
        ),
    }
}

// =========================================================================
// Aggregates honour the scope
// =========================================================================

/// GIVEN `c` scoped to `cat = 'a'` (3 of 6 rows)
/// WHEN `COUNT(*)` runs through `Database::execute_aggregate`
/// THEN it counts the 3 scoped rows, not the 6 stored.
#[test]
fn count_under_a_scope_counts_only_the_scoped_rows() {
    let (_dir, db, observer) = setup();
    assert_eq!(count(&db, "SELECT COUNT(*) AS n FROM c"), 6);

    observer.scope("c", "cat = 'a'");

    assert_eq!(count(&db, "SELECT COUNT(*) AS n FROM c"), 3);
}

/// GIVEN `c` scoped to `cat = 'a'` (amounts 10, 20, 30)
/// WHEN SUM / AVG / MIN / MAX run
/// THEN each is computed over the scoped rows alone.
#[test]
fn sum_avg_min_max_under_a_scope_see_only_the_scoped_rows() {
    let (_dir, db, observer) = setup();
    observer.scope("c", "cat = 'a'");

    let result = aggregate(
        &db,
        "SELECT SUM(amount) AS s, AVG(amount) AS m, MIN(amount) AS lo, MAX(amount) AS hi FROM c",
    )
    .expect("test: aggregate runs");

    assert_eq!(result["s"].as_f64(), Some(60.0), "SUM: {result}");
    assert_eq!(result["m"].as_f64(), Some(20.0), "AVG: {result}");
    assert_eq!(result["lo"].as_f64(), Some(10.0), "MIN: {result}");
    assert_eq!(result["hi"].as_f64(), Some(30.0), "MAX: {result}");
}

/// GIVEN `c` scoped to `cat = 'a'`
/// WHEN a `GROUP BY cat` runs
/// THEN only the `a` group comes back: neither `b` nor the NULL group.
#[test]
fn group_by_under_a_scope_returns_only_the_scoped_groups() {
    let (_dir, db, observer) = setup();
    let unscoped = aggregate(&db, "SELECT cat, COUNT(*) AS n FROM c GROUP BY cat")
        .expect("test: unscoped GROUP BY");
    assert_eq!(group_keys(&unscoped).len(), 3, "a, b and NULL: {unscoped}");

    observer.scope("c", "cat = 'a'");
    let scoped = aggregate(&db, "SELECT cat, COUNT(*) AS n FROM c GROUP BY cat")
        .expect("test: scoped GROUP BY");

    assert_eq!(group_keys(&scoped), BTreeSet::from(["\"a\"".to_string()]));
    assert_eq!(scoped[0]["n"].as_i64(), Some(3), "{scoped}");
}

/// GIVEN `c` scoped to `amount < 150` (a: 3 rows, b: 1 row)
/// WHEN `GROUP BY cat HAVING COUNT(*) > 1` runs
/// THEN HAVING sees the scoped counts, so `b` (2 stored, 1 in scope) drops out.
#[test]
fn having_under_a_scope_filters_the_scoped_groups() {
    let (_dir, db, observer) = setup();
    let sql = "SELECT cat, COUNT(*) AS n FROM c GROUP BY cat HAVING COUNT(*) > 1";
    let unscoped = aggregate(&db, sql).expect("test: unscoped HAVING");
    assert!(group_keys(&unscoped).contains("\"b\""), "{unscoped}");

    observer.scope("c", "amount < 150");
    let scoped = aggregate(&db, sql).expect("test: scoped HAVING");

    assert_eq!(group_keys(&scoped), BTreeSet::from(["\"a\"".to_string()]));
}

/// GIVEN `c` scoped to `cat = 'a'`
/// WHEN the aggregate carries its own WHERE `amount > 10`
/// THEN the two compose with AND: 2 rows (20, 30).
#[test]
fn scope_composes_with_the_aggregates_own_where() {
    let (_dir, db, observer) = setup();
    observer.scope("c", "cat = 'a'");

    assert_eq!(
        count(&db, "SELECT COUNT(*) AS n FROM c WHERE amount > 10"),
        2
    );
}

/// GIVEN the observer denies `c`
/// WHEN an aggregate reads `c`
/// THEN it fails with exactly the observer's error.
#[test]
fn deny_aborts_an_aggregate_with_the_observers_error() {
    let (_dir, db, observer) = setup();
    observer.deny("c", "no reads on c");

    assert_denied_with(
        aggregate(&db, "SELECT COUNT(*) AS n FROM c"),
        "no reads on c",
    );
    assert_denied_with(
        aggregate(&db, "SELECT cat, COUNT(*) AS n FROM c GROUP BY cat"),
        "no reads on c",
    );
}

/// GIVEN an aggregate with no FROM, whose collection comes from `_collection`
/// WHEN the observer denies that collection
/// THEN the gate is asked about the collection actually read, and denies it.
#[test]
fn an_aggregate_targeted_by_the_collection_param_is_gated_on_that_collection() {
    let (_dir, db, observer) = setup();
    observer.deny("c", "no reads on c");
    let mut query = Parser::parse("SELECT COUNT(*) AS n FROM c").expect("test: parse");
    query.select.from.clear();
    let params = HashMap::from([("_collection".to_string(), json!("c"))]);

    assert_denied_with(db.execute_aggregate(&query, &params), "no reads on c");
}

// =========================================================================
// Scalar subqueries use their own collection's decision
// =========================================================================

/// GIVEN `c` scoped to `cat = 'a'` and `d` scoped to `tenant = 'x'`
/// WHEN `COUNT(*) FROM c WHERE amount > (SELECT MAX(score) FROM d)` runs
/// THEN the subquery sees only tenant x (MAX = 7, not 1000) and the outer
///      count sees only category a: 3.
#[test]
fn a_scalar_subquery_on_another_collection_applies_that_collections_scope() {
    let (_dir, db, observer) = setup();
    let sql = "SELECT COUNT(*) AS n FROM c WHERE amount > (SELECT MAX(score) AS m FROM d)";
    assert_eq!(
        count(&db, sql),
        0,
        "unscoped: MAX(score) is 1000, no amount exceeds it"
    );

    observer.scope("c", "cat = 'a'");
    observer.scope("d", "tenant = 'x'");

    assert_eq!(count(&db, sql), 3);
}

/// GIVEN `d` scoped to `tenant = 'x'`
/// WHEN a row query on `c` filters by an aggregate subquery on `d`
/// THEN the subquery's aggregate is scoped too (MAX = 7 keeps all six rows).
#[test]
fn a_row_query_s_aggregate_subquery_applies_the_subquery_collections_scope() {
    let (_dir, db, observer) = setup();
    let sql = "SELECT * FROM c WHERE amount > (SELECT MAX(score) AS m FROM d) LIMIT 100";
    assert_eq!(rows(&db, sql).expect("test: unscoped").len(), 0);

    observer.scope("d", "tenant = 'x'");

    assert_eq!(rows(&db, sql).expect("test: scoped").len(), 6);
}

/// GIVEN the observer denies `d` and allows `c`
/// WHEN an aggregate on `c` carries a scalar subquery on `d`
/// THEN the statement fails with `d`'s denial.
#[test]
fn a_denied_subquery_collection_aborts_the_outer_statement() {
    let (_dir, db, observer) = setup();
    observer.deny("d", "no reads on d");

    assert_denied_with(
        aggregate(
            &db,
            "SELECT COUNT(*) AS n FROM c WHERE amount > (SELECT MAX(score) AS m FROM d)",
        ),
        "no reads on d",
    );
    assert_denied_with(
        rows(
            &db,
            "SELECT * FROM c WHERE amount > (SELECT AVG(score) AS m FROM d) LIMIT 10",
        ),
        "no reads on d",
    );
}

// =========================================================================
// Every collection a row query reads is gated
// =========================================================================

/// GIVEN `c` scoped to `cat = 'a'`
/// WHEN a UNION's right operand reads `c` with no WHERE
/// THEN the operand is scoped too: only the three `a` rows come back.
#[test]
fn every_operand_of_a_set_operation_is_scoped() {
    let (_dir, db, observer) = setup();
    observer.scope("c", "cat = 'a'");

    let ids = rows(
        &db,
        "SELECT * FROM c WHERE cat = 'b' UNION SELECT * FROM c LIMIT 100",
    )
    .expect("test: scoped UNION");

    assert_eq!(ids, BTreeSet::from([1, 2, 3]));
}

/// GIVEN the observer denies `d`
/// WHEN a UNION's right operand reads `d`
/// THEN the statement fails with `d`'s denial.
#[test]
fn a_denied_set_operation_operand_aborts_the_statement() {
    let (_dir, db, observer) = setup();
    observer.deny("d", "no reads on d");

    assert_denied_with(
        rows(&db, "SELECT * FROM c UNION SELECT * FROM d LIMIT 100"),
        "no reads on d",
    );
}

/// GIVEN the observer denies `d`, or scopes it
/// WHEN `c` JOINs `d`
/// THEN the denial aborts the statement, and the scope — which a JOIN's
///      secondary side cannot apply — refuses it rather than leak `d`'s rows.
#[test]
fn a_join_secondary_collection_is_gated() {
    let (_dir, db, observer) = setup();
    let sql = "SELECT * FROM c JOIN d ON c.id = d.id LIMIT 100";
    assert!(!rows(&db, sql).expect("test: unscoped JOIN").is_empty());

    observer.deny("d", "no reads on d");
    assert_denied_with(rows(&db, sql), "no reads on d");

    observer.scope("d", "tenant = 'x'");
    assert_refused_unscoped(rows(&db, sql));
}

/// GIVEN `c` scoped to `cat = 'a'`
/// WHEN a dense `NEAR` search runs through `execute_query`
/// THEN only scoped rows come back.
#[test]
fn a_vector_search_is_scoped() {
    let (_dir, db, observer) = setup();
    observer.scope("c", "cat = 'a'");
    let query =
        Parser::parse("SELECT * FROM c WHERE vector NEAR $v LIMIT 100").expect("test: parse NEAR");
    let params = HashMap::from([("v".to_string(), json!([1.0, 0.0, 0.0, 0.0]))]);

    let ids: BTreeSet<u64> = db
        .execute_query(&query, &params)
        .expect("test: scoped NEAR")
        .into_iter()
        .map(|r| r.point.id)
        .collect();

    assert_eq!(ids, BTreeSet::from([1, 2, 3]));
}

// =========================================================================
// Graph reads: MATCH, @collection enrichment, SELECT EDGES
// =========================================================================

/// Creates graph collection `g` (two nodes, one edge) and returns it.
fn seed_graph(db: &Database) {
    let query =
        Parser::parse("CREATE GRAPH COLLECTION g (dimension = 4, metric = 'cosine') SCHEMALESS")
            .expect("test: parse CREATE GRAPH");
    db.execute_query(&query, &HashMap::new())
        .expect("test: create g");
    let g = db.get_graph_collection("g").expect("test: get g");
    g.upsert_node_payload(1, &json!({"_labels": ["Person"], "name": "Ann"}))
        .expect("test: node 1");
    g.upsert_node_payload(2, &json!({"_labels": ["Person"], "name": "Bo"}))
        .expect("test: node 2");
    g.add_edge(GraphEdge::new(1, 1, 2, "KNOWS").expect("test: edge"))
        .expect("test: add edge");
}

fn match_on(db: &Database, sql: &str, collection: &str) -> velesdb_core::Result<usize> {
    let query = Parser::parse(sql).expect("test: parse MATCH");
    let params = HashMap::from([("_collection".to_string(), json!(collection))]);
    db.execute_query(&query, &params).map(|r| r.len())
}

/// GIVEN a MATCH whose collection comes from `_collection`
/// WHEN the observer denies that collection
/// THEN the gate is asked about it and the traversal is denied.
#[test]
fn a_match_targeted_by_the_collection_param_is_gated_on_that_collection() {
    let (_dir, db, observer) = setup();
    seed_graph(&db);
    let sql = "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10";
    assert_eq!(match_on(&db, sql, "g").expect("test: allowed MATCH"), 1);

    observer.deny("g", "no reads on g");

    assert_denied_with(match_on(&db, sql, "g"), "no reads on g");
    assert!(
        observer.asked().iter().any(|c| c == "g"),
        "{:?}",
        observer.asked()
    );
}

/// GIVEN `g` scoped
/// WHEN a MATCH traverses `g` (a traversal has no metadata-filter channel)
/// THEN the statement is refused rather than run unscoped.
#[test]
fn a_scoped_match_is_refused_rather_than_run_unscoped() {
    let (_dir, db, observer) = setup();
    seed_graph(&db);
    observer.scope("g", "name = 'Ann'");

    assert_refused_unscoped(match_on(
        &db,
        "MATCH (a:Person)-[:KNOWS]->(b:Person) RETURN a, b LIMIT 10",
        "g",
    ));
}

/// GIVEN the observer denies `c`
/// WHEN a MATCH on `g` enriches its nodes from `c` through `@c`
/// THEN the statement fails with `c`'s denial instead of merging `c`'s payloads.
#[test]
fn a_match_collection_annotation_is_gated() {
    let (_dir, db, observer) = setup();
    seed_graph(&db);
    observer.deny("c", "no reads on c");

    assert_denied_with(
        match_on(
            &db,
            "MATCH (a:Person)-[:KNOWS]->(b:Person@c) RETURN a, b LIMIT 10",
            "g",
        ),
        "no reads on c",
    );
}

/// GIVEN the observer denies `g`, or scopes it
/// WHEN `SELECT EDGES FROM g` runs
/// THEN the denial aborts it and the scope (edges carry no payload to
///      filter) refuses it.
#[test]
fn select_edges_is_gated() {
    let (_dir, db, observer) = setup();
    seed_graph(&db);
    let sql = "SELECT EDGES FROM g";
    assert_eq!(rows(&db, sql).expect("test: allowed SELECT EDGES").len(), 1);

    observer.deny("g", "no reads on g");
    assert_denied_with(rows(&db, sql), "no reads on g");

    observer.scope("g", "name = 'Ann'");
    assert_refused_unscoped(rows(&db, sql));
}

// =========================================================================
// Negative control: a scope never widens
// =========================================================================

/// GIVEN a scope that matches every stored row (`amount > 0`)
/// WHEN an aggregate narrows with its own WHERE `cat = 'b'`
/// THEN the scope cannot widen it back: still 2.
/// AND a scope matching nothing yields 0 and no groups.
#[test]
fn a_scope_can_never_widen_an_aggregate() {
    let (_dir, db, observer) = setup();
    observer.scope("c", "amount > 0");
    assert_eq!(count(&db, "SELECT COUNT(*) AS n FROM c WHERE cat = 'b'"), 2);

    observer.scope("c", "cat = 'b' OR cat = 'a' OR amount > 0");
    assert_eq!(count(&db, "SELECT COUNT(*) AS n FROM c WHERE cat = 'b'"), 2);

    observer.scope("c", "cat = 'nobody'");
    assert_eq!(count(&db, "SELECT COUNT(*) AS n FROM c"), 0);
    let groups = aggregate(&db, "SELECT cat, COUNT(*) AS n FROM c GROUP BY cat")
        .expect("test: empty-scope GROUP BY");
    assert!(group_keys(&groups).is_empty(), "{groups}");
}

/// Predicates the property draws its base WHERE and its scope from.
const PREDICATES: [&str; 6] = [
    "cat = 'a'",
    "cat = 'b'",
    "amount > 15",
    "amount <= 200",
    "cat = 'a' OR amount > 150",
    "amount > 5000",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// For any base WHERE (or none) and any scope `F`, the scoped `COUNT(*)`
    /// equals the unscoped `COUNT(*)` of `(base) AND (F)` and never exceeds
    /// the unscoped `COUNT(*)` of the base alone.
    #[test]
    fn a_scoped_count_is_the_count_of_the_intersection(
        base in proptest::option::of(0usize..PREDICATES.len()),
        scope in 0usize..PREDICATES.len(),
    ) {
        let (_dir, db, observer) = setup();
        let base_sql = match base {
            Some(i) => format!("SELECT COUNT(*) AS n FROM c WHERE {}", PREDICATES[i]),
            None => "SELECT COUNT(*) AS n FROM c".to_string(),
        };
        let intersect_sql = match base {
            Some(i) => format!(
                "SELECT COUNT(*) AS n FROM c WHERE ({}) AND ({})",
                PREDICATES[i], PREDICATES[scope]
            ),
            None => format!("SELECT COUNT(*) AS n FROM c WHERE {}", PREDICATES[scope]),
        };

        let unscoped = count(&db, &base_sql);
        let expected = count(&db, &intersect_sql);
        observer.scope("c", PREDICATES[scope]);
        let scoped = count(&db, &base_sql);
        observer.allow_all();

        prop_assert!(scoped <= unscoped, "scope widened: {scoped} > {unscoped}");
        prop_assert_eq!(scoped, expected, "{} under scope {}", base_sql, PREDICATES[scope]);
    }
}
