//! Read-path control-plane gate — scopes the gate must refuse outright.
//!
//! The `VelesQL -> filter` lowering is total by design: the query engine
//! pre-extracts engine-handled conditions before lowering a WHERE clause.
//! An observer scope cannot ride that totality — a conjunct it cannot
//! express as a row-level metadata predicate must refuse (`VELES-009`),
//! never silently narrow by a different predicate or not at all.
//!
//! * A `MATCH` scope conjunct lowers to substring `Contains` — full-text
//!   semantics narrowed by the wrong predicate. `CONTAINS_TEXT` lowers
//!   faithfully (it already is a strict substring predicate) and stays legal.
//! * A `$param` or subquery value in a scope is not a policy value — it
//!   would bind to the caller's params or lower to NULL.
//! * A scope column qualified with a JOIN table (`other.col`) would be
//!   pushdown-classified onto the join side, where a LEFT JOIN keeps every
//!   base row anyway — the base leaks unscoped.

#![cfg(feature = "persistence")]

use std::collections::HashMap;
use std::sync::Arc;

use tempfile::TempDir;
use velesdb_core::velesql::Parser;
use velesdb_core::{
    AccessDecision, AccessScope, Database, DatabaseObserver, DistanceMetric, Point,
    QueryAccessContext, QueryOperationKind,
};

/// Observer that returns `AllowWithScope` carrying the configured filter.
struct ScopeObserver {
    filter: Option<velesdb_core::velesql::Condition>,
}

impl ScopeObserver {
    fn scoped(sql_where: &str) -> Self {
        let filter = Parser::parse(&format!("SELECT * FROM c WHERE {sql_where}"))
            .expect("test: scope predicate parses")
            .select
            .where_clause;
        Self { filter }
    }
}

impl DatabaseObserver for ScopeObserver {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        // Scope only `c`: a scope on the JOIN's secondary `d` is itself a
        // refusal — answering it here would mask the bug under test.
        #[allow(clippy::field_reassign_with_default)] // `AccessScope` is non_exhaustive
        let mut scope = AccessScope::default();
        scope.filter = (ctx.collection == "c")
            .then(|| self.filter.clone())
            .flatten();
        Ok(AccessDecision::AllowWithScope(scope))
    }
}

fn db_with(observer: impl DatabaseObserver + 'static) -> (TempDir, Database) {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), Arc::new(observer)).expect("test: open db");
    db.create_vector_collection("c", 4, DistanceMetric::Cosine)
        .expect("test: create c");
    db.create_vector_collection("d", 4, DistanceMetric::Cosine)
        .expect("test: create d");
    db.get_vector_collection("c")
        .expect("test: get c")
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0, 0.0],
            Some(serde_json::json!({"tenant": "a", "body": "keep me"})),
        )])
        .expect("test: upsert");
    (dir, db)
}

/// GIVEN an observer that scopes reads by a full-text `MATCH` conjunct
/// WHEN `authorize_read` resolves it
/// THEN it refuses — substring `Contains` is not `MATCH`.
#[test]
fn a_match_scope_is_refused_not_degraded() {
    let (_dir, db) = db_with(ScopeObserver::scoped("body MATCH 'keep'"));
    let outcome = db.authorize_read("c", QueryOperationKind::Select, None, None);
    assert!(
        outcome.is_err(),
        "a MATCH scope must be refused, got {outcome:?}"
    );
}

/// GIVEN an observer that scopes reads by `CONTAINS_TEXT` — already a strict
/// substring metadata predicate
/// WHEN `authorize_read` resolves it
/// THEN the scope lowers faithfully and applies.
#[test]
fn a_contains_text_scope_still_applies() {
    let (_dir, db) = db_with(ScopeObserver::scoped("body CONTAINS_TEXT 'keep'"));
    let outcome = db
        .authorize_read("c", QueryOperationKind::Select, None, None)
        .expect("test: CONTAINS_TEXT lowers faithfully");
    let filter = outcome.expect("test: scope filter present");
    assert!(filter.matches(&serde_json::json!({"body": "keep me"})));
    assert!(!filter.matches(&serde_json::json!({"body": "drop me"})));
}

/// GIVEN an observer that scopes reads by a `$param` placeholder
/// WHEN `authorize_read` resolves it
/// THEN it refuses — a scope must be self-contained, never bound by the caller.
#[test]
fn a_param_scope_is_refused() {
    let (_dir, db) = db_with(ScopeObserver::scoped("tenant = $t"));
    let outcome = db.authorize_read("c", QueryOperationKind::Select, None, None);
    assert!(
        outcome.is_err(),
        "a $param scope must be refused, got {outcome:?}"
    );
}

/// GIVEN an observer that scopes reads by a subquery value
/// WHEN `authorize_read` resolves it
/// THEN it refuses — a subquery has no resolution context at the gate.
#[test]
fn a_subquery_scope_is_refused() {
    let (_dir, db) = db_with(ScopeObserver::scoped("id > (SELECT MAX(id) AS m FROM d)"));
    let outcome = db.authorize_read("c", QueryOperationKind::Select, None, None);
    assert!(
        outcome.is_err(),
        "a subquery scope must be refused, got {outcome:?}"
    );
}

/// GIVEN an observer that scopes `c` by a JOIN-table-qualified column
/// (`d.tenant`) and a statement reading `c LEFT JOIN d`
/// WHEN the gate folds the scope into the base WHERE
/// THEN it refuses rather than push the scope to the join side — a LEFT JOIN
/// keeps every base row, so the base would be returned unscoped.
#[test]
fn a_join_qualified_scope_is_refused() {
    let (_dir, db) = db_with(ScopeObserver::scoped("d.tenant = 'a'"));
    let query = Parser::parse("SELECT * FROM c LEFT JOIN d ON c.id = d.id WHERE cat = 'x'")
        .expect("test: join query parses");

    let outcome = db.execute_query(&query, &HashMap::new());
    assert!(
        outcome.is_err(),
        "a JOIN-qualified scope must be refused, got {outcome:?}"
    );
}

/// GIVEN the same observer
/// WHEN the statement reads `c` with no JOIN
/// THEN the unqualified scope applies normally — the refusal above is about
/// the JOIN misclassification, not the scope itself.
#[test]
fn an_unqualified_scope_still_applies_with_a_join() {
    let (_dir, db) = db_with(ScopeObserver::scoped("tenant = 'a'"));
    let query = Parser::parse("SELECT * FROM c LEFT JOIN d ON c.id = d.id LIMIT 100")
        .expect("test: join query parses");
    let results = db
        .execute_query(&query, &HashMap::new())
        .expect("test: unqualified scope applies");
    assert_eq!(results.len(), 1);
}

/// GIVEN a database whose observer denies every read
/// WHEN `SemanticMemory::count`/`is_empty` run — cardinality of the same
/// `point_count` every other surface gates
/// THEN they fail with the denial.
#[test]
fn semantic_memory_count_and_is_empty_are_gated() {
    struct DenyAll;
    impl DatabaseObserver for DenyAll {
        fn on_query_request(
            &self,
            _ctx: &QueryAccessContext,
        ) -> velesdb_core::Result<AccessDecision> {
            Ok(AccessDecision::Deny(velesdb_core::Error::Query(
                "read denied by test policy".to_string(),
            )))
        }
    }
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), Arc::new(DenyAll)).expect("test: open db");
    let semantic =
        velesdb_core::agent::SemanticMemory::new_from_db(Arc::new(db), 4).expect("test: memory");

    assert!(
        semantic.count().is_err(),
        "count() returns the collection cardinality and must fail under a denied read"
    );
    assert!(
        semantic.is_empty().is_err(),
        "is_empty() derives from the same cardinality and must fail too"
    );
}

/// GIVEN a database whose observer records `on_query` calls
/// WHEN a MATCH addressed through `_collection` executes
/// THEN the reported collection is the gate's target — not the empty `FROM`.
#[test]
fn on_query_reports_the_match_target_collection() {
    use parking_lot::Mutex;
    struct Recorder {
        seen: Mutex<Vec<String>>,
    }
    impl DatabaseObserver for Recorder {
        fn on_query(&self, collection: &str, _duration_us: u64) {
            self.seen.lock().push(collection.to_string());
        }
    }
    let recorder = Arc::new(Recorder {
        seen: Mutex::new(Vec::new()),
    });
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), recorder.clone()).expect("test: open db");
    db.create_vector_collection("c", 4, DistanceMetric::Cosine)
        .expect("test: create c");
    db.get_vector_collection("c")
        .expect("test: get c")
        .upsert(vec![Point::new(1, vec![1.0, 0.0, 0.0, 0.0], None)])
        .expect("test: upsert");

    let query = Parser::parse("MATCH (n) RETURN n LIMIT 10").expect("test: match parses");
    let params = HashMap::from([("_collection".to_string(), serde_json::json!("c"))]);
    db.execute_query(&query, &params).expect("test: match runs");

    let seen = recorder.seen.lock().clone();
    assert_eq!(seen, vec!["c".to_string()], "on_query must see the target");
}
