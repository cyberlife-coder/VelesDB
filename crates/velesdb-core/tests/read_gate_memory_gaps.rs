//! Read-path control-plane gate — adversarial coverage of reads the gate
//! misses today.
//!
//! Three holes, each pinned by a test that fails until the read consults the
//! gate:
//!
//! * `SemanticMemory::edge_count` resolves its collection and reads its edge
//!   count without ever calling `authorize_memory_read` — the gate block that
//!   was written for it (operation `GraphTraversal`, context string
//!   `"semantic memory edge_count"`) sits inside `clear()` instead.
//! * `EpisodicMemory::get_with_embedding` resolves the collection and calls
//!   `collection.get(&[id])` directly, while every sibling read (`recent`,
//!   `older_than`, `serialize`, `recall_similar`) is gated.
//! * `Database::authorize_read` hands an observer scope filter to
//!   `scope_to_core_filter`, whose `velesql::Condition -> filter::Condition`
//!   lowering turns a `VectorSearch`/`Similarity`/`GraphMatch` conjunct into
//!   `Condition::And { conditions: vec![] }` — a tautology that matches every
//!   payload. A scope the leaf cannot express silently degrades to
//!   "match all" instead of refusing (`VELES-009`), so the caller that
//!   AND-composes it runs unscoped.

#![cfg(feature = "persistence")]

use std::sync::Arc;

use tempfile::TempDir;
use velesdb_core::agent::{EpisodicMemory, SemanticMemory};
use velesdb_core::velesql::Parser;
use velesdb_core::{
    AccessDecision, AccessScope, Database, DatabaseObserver, Error, QueryAccessContext,
};

/// Observer that denies every read it is asked about.
struct DenyAll;

impl DatabaseObserver for DenyAll {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let _ = ctx;
        Ok(AccessDecision::Deny(Error::Query(
            "read denied by test policy".to_string(),
        )))
    }
}

/// Observer returning a scope whose single conjunct is a `similarity()`
/// predicate — a `VelesQL` condition the `filter::Condition` language cannot
/// express.
struct SimilarityScope;

impl DatabaseObserver for SimilarityScope {
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let _ = ctx;
        let filter =
            Parser::parse("SELECT * FROM c WHERE similarity(vector, [1.0, 0.0, 0.0, 0.0]) > 0.5")
                .expect("test: scope predicate parses")
                .select
                .where_clause;
        #[allow(clippy::field_reassign_with_default)] // `AccessScope` is non_exhaustive
        let mut scope = AccessScope::default();
        scope.filter = filter;
        Ok(AccessDecision::AllowWithScope(scope))
    }
}

/// GIVEN a database whose observer denies every read
/// WHEN `SemanticMemory::edge_count` runs
/// THEN it fails with the denial — it reads the collection's edge store.
#[test]
fn semantic_memory_edge_count_is_gated() {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), Arc::new(DenyAll)).expect("test: open db");
    let semantic = SemanticMemory::new_from_db(Arc::new(db), 4).expect("test: memory");

    let result = semantic.edge_count();
    assert!(
        result.is_err(),
        "edge_count reads the collection and must fail under a denied read, \
         got {result:?}"
    );
}

/// GIVEN a database whose observer denies every read
/// WHEN `EpisodicMemory::get_with_embedding` runs
/// THEN it fails with the denial — it is a point payload lookup.
#[test]
fn episodic_memory_get_with_embedding_is_gated() {
    let dir = TempDir::new().expect("test: tempdir");
    let db = Database::open_with_observer(dir.path(), Arc::new(DenyAll)).expect("test: open db");
    let episodic = EpisodicMemory::new_from_db(Arc::new(db), 4).expect("test: memory");
    episodic
        .record(1, "an event", 1_700_000_000, Some(&[1.0, 0.0, 0.0, 0.0]))
        .expect("test: record is a write, not a gated read");

    let result = episodic.get_with_embedding(1);
    assert!(
        result.is_err(),
        "get_with_embedding reads a point payload and must fail under a denied \
         read, got {result:?}"
    );
}

/// GIVEN an observer that scopes every read by a `similarity()` conjunct —
/// which the `filter::Condition` language cannot express
/// WHEN the raw read gate resolves it
/// THEN it refuses rather than hand back a filter that matches every payload.
///
/// Today `scope_to_core_filter` lowers the conjunct to an empty `And` — a
/// tautology — so `authorize_read` returns `Ok(Some(f))` with
/// `f.matches(anything) == true`: callers that AND-compose it run unscoped
/// and callers that "fail closed on `Some`" refuse a scope that was never
/// really applied. Either way the observer's scope is silently dropped.
#[test]
fn raw_gate_refuses_a_scope_it_cannot_express() {
    let dir = TempDir::new().expect("test: tempdir");
    let db =
        Database::open_with_observer(dir.path(), Arc::new(SimilarityScope)).expect("test: open db");

    let outcome = db.authorize_read(
        "c",
        velesdb_core::observer::QueryOperationKind::VectorSearch,
        None,
        None,
    );

    match outcome {
        Err(_) => {} // fail closed, as VELES-009 requires
        Ok(None) => panic!("an inexpressible scope must not read as an unscoped allow"),
        Ok(Some(filter)) => {
            assert!(
                !filter.matches(&serde_json::json!({"tenant": "someone-else"})),
                "a scope the leaf cannot express was lowered to a match-all \
                 tautology instead of being refused"
            );
        }
    }
}
