//! `PyObserver` — bridges core [`DatabaseObserver`] lifecycle hooks to a single
//! Python callable.
//!
//! The Python SDK exposes the four *notify* callbacks (create/delete/upsert/
//! query) plus the read-path **veto** hook (`query_request`): the callback may
//! refuse a read by returning `False` or a string reason, so an SDK embedder can
//! enforce governance on every gated read (VelesQL `SELECT`/`MATCH`). The DDL/DML
//! veto hooks (`on_ddl_request` / `on_dml_mutation_request`) keep the trait's
//! default-allow behavior and remain unexposed.
//!
//! # GIL safety
//!
//! Core fires every callback *after* dropping the collection write guard, so
//! re-acquiring the GIL here cannot deadlock the core-lock against the GIL.
//! Notify callbacks swallow any [`PyErr`] (logged, never propagated) and never
//! panic — the [`DatabaseObserver`] contract forbids panicking. The
//! `query_request` veto also never panics: a raised callback error is logged and
//! treated as *allow* (fail-open on observer error, never break a read on a bug
//! in user policy code — an explicit `False`/reason is the only way to deny).

use pyo3::prelude::*;
use pyo3::types::PyDict;
use velesdb_core::collection::CollectionType;
use velesdb_core::observer::{AccessDecision, QueryAccessContext, QueryOperationKind};
use velesdb_core::{DatabaseObserver, Error};

/// A [`DatabaseObserver`] that forwards lifecycle events to one Python callable.
///
/// The callback is invoked as `callback(event, **fields)`, where `event` is one
/// of `"collection_created"`, `"collection_deleted"`, `"upsert"`, or `"query"`,
/// and the keyword fields carry the event payload:
///
/// | event                | keyword fields              |
/// |----------------------|-----------------------------|
/// | `collection_created` | `name`, `kind`              |
/// | `collection_deleted` | `name`                      |
/// | `upsert`             | `collection`, `point_count` |
/// | `query`              | `collection`, `duration_us` |
///
/// `kind` is one of `"vector"`, `"metadata"`, `"graph"`, or `"unknown"`
/// (the last guards against a future [`CollectionType`] variant).
pub struct PyObserver {
    /// User-supplied Python callable. `Py<PyAny>` is `Send + Sync`, satisfying
    /// the `DatabaseObserver: Send + Sync` bound.
    cb: Py<PyAny>,
}

impl PyObserver {
    /// Wraps a Python callable as a core observer.
    pub fn new(cb: Py<PyAny>) -> Self {
        Self { cb }
    }

    /// Invokes the callback as `callback(event, **fields)`, swallowing any
    /// `PyErr` so an observer side effect can never break a core operation.
    ///
    /// Takes the already-held GIL token `py` so callers acquire the GIL exactly
    /// once per event (build the `PyDict`, then dispatch under the same token).
    fn dispatch(&self, py: Python<'_>, event: &str, fields: &Bound<'_, PyDict>) {
        if let Err(err) = self.cb.bind(py).call((event,), Some(fields)) {
            eprintln!("[velesdb] observer callback '{event}' raised; ignoring: {err}");
        }
    }
}

/// Maps a [`CollectionType`] to a stable kind string. The `_` arm keeps this
/// total against the `#[non_exhaustive]` enum — it must never panic.
fn collection_kind(kind: &CollectionType) -> &'static str {
    match kind {
        CollectionType::Vector { .. } => "vector",
        CollectionType::MetadataOnly => "metadata",
        CollectionType::Graph { .. } => "graph",
        _ => "unknown",
    }
}

/// Maps a [`QueryOperationKind`] to a stable operation string for the callback.
/// The `_` arm keeps this total against the `#[non_exhaustive]` enum.
fn operation_str(op: QueryOperationKind) -> &'static str {
    match op {
        QueryOperationKind::VectorSearch => "vector_search",
        QueryOperationKind::TextSearch => "text_search",
        QueryOperationKind::HybridSearch => "hybrid_search",
        QueryOperationKind::GraphTraversal => "graph_traversal",
        QueryOperationKind::Select => "select",
        _ => "unknown",
    }
}

/// Interprets a `query_request` callback return value as an [`AccessDecision`]:
///
/// * `None` or `True` (or any other truthy object) → [`AccessDecision::Allow`]
///   — so an existing notify-only callback that ignores the event and returns
///   `None` keeps allowing every read (backward compatible);
/// * `False` → [`AccessDecision::Deny`] with a default reason;
/// * a `str` → `Deny` carrying that string as the human-readable reason.
fn interpret_decision(ret: &Bound<'_, PyAny>) -> AccessDecision {
    if ret.is_none() {
        return AccessDecision::Allow;
    }
    // A string is a deny *reason*. Check before bool so `""`—an empty reason—
    // still denies rather than being coerced to a falsey bool.
    if let Ok(reason) = ret.extract::<String>() {
        return AccessDecision::Deny(Error::Query(format!("read denied by observer: {reason}")));
    }
    if let Ok(allow) = ret.extract::<bool>() {
        return if allow {
            AccessDecision::Allow
        } else {
            AccessDecision::Deny(Error::Query("read denied by observer policy".to_string()))
        };
    }
    AccessDecision::Allow
}

impl DatabaseObserver for PyObserver {
    fn on_collection_created(&self, name: &str, kind: &CollectionType) {
        let kind = collection_kind(kind);
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("name", name).is_err() || fields.set_item("kind", kind).is_err() {
                return;
            }
            self.dispatch(py, "collection_created", &fields);
        });
    }

    fn on_collection_deleted(&self, name: &str) {
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("name", name).is_err() {
                return;
            }
            self.dispatch(py, "collection_deleted", &fields);
        });
    }

    fn on_upsert(&self, collection: &str, point_count: usize) {
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", collection).is_err()
                || fields.set_item("point_count", point_count).is_err()
            {
                return;
            }
            self.dispatch(py, "upsert", &fields);
        });
    }

    fn on_query(&self, collection: &str, duration_us: u64) {
        Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", collection).is_err()
                || fields.set_item("duration_us", duration_us).is_err()
            {
                return;
            }
            self.dispatch(py, "query", &fields);
        });
    }

    /// Read-path veto. Invokes the callback as
    /// `callback("query_request", collection=…, operation=…, principal=…, tenant=…)`
    /// and maps its return value through [`interpret_decision`]: `None`/`True`
    /// allow, `False`/str deny. A callback that raises, or a field-population
    /// failure, allows the read (fail-open on error so a bug in user policy code
    /// never breaks a query — only an explicit refusal denies). Fires on every
    /// gated read (VelesQL `SELECT`/`MATCH`) so an SDK embedder can enforce
    /// governance, closing the "notify-only" gap (CORE-5).
    fn on_query_request(&self, ctx: &QueryAccessContext) -> velesdb_core::Result<AccessDecision> {
        let decision = Python::attach(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", ctx.collection).is_err()
                || fields
                    .set_item("operation", operation_str(ctx.operation))
                    .is_err()
                || fields.set_item("principal", ctx.principal).is_err()
                || fields.set_item("tenant", ctx.tenant_hint).is_err()
            {
                return AccessDecision::Allow;
            }
            match self.cb.bind(py).call(("query_request",), Some(&fields)) {
                Ok(ret) => interpret_decision(&ret),
                Err(err) => {
                    eprintln!(
                        "[velesdb] observer callback 'query_request' raised; allowing: {err}"
                    );
                    AccessDecision::Allow
                }
            }
        });
        Ok(decision)
    }
}
