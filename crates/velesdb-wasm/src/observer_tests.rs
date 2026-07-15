//! Tests for the wasm-local read-path observer gate (audit F-5.4, #1392).
//!
//! Run on the native host target (like the other `velesdb-wasm` tests). They
//! deliberately avoid the `JsValue`-returning `search`/`executeQuery` FFI
//! methods (which panic off `wasm32`) and exercise the gate through the
//! `String`-error seams: [`DatabaseInner::check_query_access`],
//! [`crate::velesql_exec::execute`], and the shared [`gate`] helper the
//! `WasmCollectionHandle::search` path uses.

use std::rc::Rc;

use super::{
    gate, WasmAccessDecision, WasmObserver, WasmQueryAccessContext, WasmQueryOperationKind,
};
use crate::database::{DatabaseInner, WasmDatabase};

/// Observer that denies a configured set of operations.
struct DenyObserver {
    deny: Vec<WasmQueryOperationKind>,
}

impl DenyObserver {
    fn new(deny: Vec<WasmQueryOperationKind>) -> Rc<Self> {
        Rc::new(Self { deny })
    }
}

impl WasmObserver for DenyObserver {
    fn on_query_request(&self, ctx: &WasmQueryAccessContext<'_>) -> WasmAccessDecision {
        if self.deny.contains(&ctx.operation) {
            WasmAccessDecision::Deny(format!(
                "denied {:?} on '{}'",
                ctx.operation, ctx.collection
            ))
        } else {
            WasmAccessDecision::Allow
        }
    }
}

/// Observer using every default method — behaves as if absent (allow-all).
struct DefaultObserver;
impl WasmObserver for DefaultObserver {}

fn seeded_metadata_db() -> DatabaseInner {
    let mut inner = DatabaseInner::new();
    inner
        .create_metadata_collection("docs")
        .expect("test: create metadata collection");
    crate::velesql_exec::execute(
        &mut inner,
        "INSERT INTO docs (id, n) VALUES (1, 10), (2, 20), (3, 30)",
        None,
    )
    .expect("test: seed rows");
    inner
}

// --- gate helper: zero-overhead / allow-by-default ------------------------

#[test]
fn gate_without_observer_is_ok_for_every_op() {
    for op in [
        WasmQueryOperationKind::VectorSearch,
        WasmQueryOperationKind::Select,
        WasmQueryOperationKind::GraphTraversal,
    ] {
        assert!(gate(None, "docs", op).is_ok());
    }
}

#[test]
fn gate_with_default_observer_allows() {
    let obs: Rc<dyn WasmObserver> = Rc::new(DefaultObserver);
    assert!(gate(Some(&obs), "docs", WasmQueryOperationKind::VectorSearch).is_ok());
}

#[test]
fn gate_deny_propagates_message() {
    let obs: Rc<dyn WasmObserver> = DenyObserver::new(vec![WasmQueryOperationKind::VectorSearch]);
    let err = gate(Some(&obs), "docs", WasmQueryOperationKind::VectorSearch)
        .expect_err("test: should be denied");
    assert!(err.contains("denied"));
    assert!(err.contains("docs"));
}

// --- DatabaseInner::check_query_access ------------------------------------

#[test]
fn check_query_access_no_observer_allows_all() {
    let inner = DatabaseInner::new();
    assert!(inner
        .check_query_access("docs", WasmQueryOperationKind::Select)
        .is_ok());
    assert!(inner
        .check_query_access("docs", WasmQueryOperationKind::VectorSearch)
        .is_ok());
}

#[test]
fn check_query_access_denies_only_targeted_op() {
    let mut inner = DatabaseInner::new();
    inner.register_observer(DenyObserver::new(vec![
        WasmQueryOperationKind::VectorSearch,
    ]));
    assert!(
        inner
            .check_query_access("docs", WasmQueryOperationKind::VectorSearch)
            .is_err(),
        "targeted op is denied"
    );
    assert!(
        inner
            .check_query_access("docs", WasmQueryOperationKind::Select)
            .is_ok(),
        "non-targeted op is allowed"
    );
}

// --- VelesQL SELECT read path ---------------------------------------------

#[test]
fn select_denied_returns_error_and_no_rows() {
    let mut inner = seeded_metadata_db();
    inner.register_observer(DenyObserver::new(vec![WasmQueryOperationKind::Select]));
    let err = crate::velesql_exec::execute(&mut inner, "SELECT * FROM docs", None)
        .expect_err("test: select must be denied");
    assert!(err.contains("denied"), "carries the denial message: {err}");
}

#[test]
fn select_allowed_returns_rows() {
    let mut inner = seeded_metadata_db();
    inner.register_observer(Rc::new(DefaultObserver));
    let r = crate::velesql_exec::execute(&mut inner, "SELECT * FROM docs", None)
        .expect("test: select allowed");
    assert_eq!(r.row_count(), 3);
}

#[test]
fn select_without_observer_is_unchanged() {
    // Zero-overhead path: identical behavior to pre-observer code.
    let mut inner = seeded_metadata_db();
    let r =
        crate::velesql_exec::execute(&mut inner, "SELECT * FROM docs", None).expect("test: select");
    assert_eq!(r.row_count(), 3);
}

#[test]
fn set_operation_operands_are_each_gated() {
    // UNION operands re-enter velesql_select::execute; a Select-deny must
    // block the compound too.
    let mut inner = seeded_metadata_db();
    inner.register_observer(DenyObserver::new(vec![WasmQueryOperationKind::Select]));
    let err = crate::velesql_exec::execute(
        &mut inner,
        "SELECT * FROM docs UNION SELECT * FROM docs",
        None,
    )
    .expect_err("test: union must be denied");
    assert!(err.contains("denied"));
}

// --- Graph read paths ------------------------------------------------------

#[test]
fn select_edges_denied_before_store_lookup() {
    // Deny short-circuits ahead of the graph-store lookup: the error is the
    // denial, not "Graph not found".
    let mut inner = DatabaseInner::new();
    inner.register_observer(DenyObserver::new(vec![
        WasmQueryOperationKind::GraphTraversal,
    ]));
    let err = crate::velesql_exec::execute(&mut inner, "SELECT EDGES FROM social", None)
        .expect_err("test: select edges denied");
    assert!(err.contains("denied"), "denial wins over not-found: {err}");
}

#[test]
fn select_edges_allowed_passes_gate_to_store_lookup() {
    // With an allowing observer the gate is transparent: execution proceeds
    // to the store lookup, which reports the missing graph (proving the gate
    // did not short-circuit).
    let mut inner = DatabaseInner::new();
    inner.register_observer(Rc::new(DefaultObserver));
    let err = crate::velesql_exec::execute(&mut inner, "SELECT EDGES FROM social", None)
        .expect_err("test: missing graph");
    assert!(err.contains("not found"), "gate is transparent: {err}");
}

// --- WasmCollectionHandle observer snapshot -------------------------------

#[test]
fn handle_snapshots_observer_registered_before_handout() {
    let mut db = WasmDatabase::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    db.register_observer(DenyObserver::new(vec![
        WasmQueryOperationKind::VectorSearch,
    ]));
    let handle = db.get_collection("docs").expect("test: handle");
    assert!(
        handle.has_observer(),
        "handle obtained after registration is governed"
    );
}

#[test]
fn handle_obtained_before_registration_has_no_observer() {
    let mut db = WasmDatabase::new();
    db.create_collection("docs", 4, "cosine")
        .expect("test: create");
    let handle = db.get_collection("docs").expect("test: handle");
    db.register_observer(DenyObserver::new(vec![
        WasmQueryOperationKind::VectorSearch,
    ]));
    assert!(
        !handle.has_observer(),
        "handle snapshots observer state at hand-out time"
    );
}

// --- register replaces -----------------------------------------------------

#[test]
fn register_observer_replaces_previous() {
    let mut inner = DatabaseInner::new();
    inner.register_observer(Rc::new(DefaultObserver));
    assert!(inner
        .check_query_access("docs", WasmQueryOperationKind::Select)
        .is_ok());
    inner.register_observer(DenyObserver::new(vec![WasmQueryOperationKind::Select]));
    assert!(inner
        .check_query_access("docs", WasmQueryOperationKind::Select)
        .is_err());
}
