//! `PyObserver` — bridges core [`DatabaseObserver`] lifecycle hooks to a single
//! Python callable.
//!
//! The Python SDK exposes only the four *notify* callbacks (create/delete/
//! upsert/query). The two veto hooks (`on_ddl_request` / `on_dml_mutation_request`)
//! keep the trait's default-allow behavior and are intentionally **not** exposed —
//! policy/RBAC enforcement is out of scope for the open SDK.
//!
//! # GIL safety
//!
//! Core fires every callback *after* dropping the collection write guard, so
//! re-acquiring the GIL here cannot deadlock the core-lock against the GIL.
//! Each callback still swallows any [`PyErr`] (logged, never propagated) and
//! never panics — the [`DatabaseObserver`] contract forbids panicking.

use pyo3::prelude::*;
use pyo3::types::PyDict;
use velesdb_core::collection::CollectionType;
use velesdb_core::DatabaseObserver;

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

impl DatabaseObserver for PyObserver {
    fn on_collection_created(&self, name: &str, kind: &CollectionType) {
        let kind = collection_kind(kind);
        Python::with_gil(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("name", name).is_err() || fields.set_item("kind", kind).is_err() {
                return;
            }
            self.dispatch(py, "collection_created", &fields);
        });
    }

    fn on_collection_deleted(&self, name: &str) {
        Python::with_gil(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("name", name).is_err() {
                return;
            }
            self.dispatch(py, "collection_deleted", &fields);
        });
    }

    fn on_upsert(&self, collection: &str, point_count: usize) {
        Python::with_gil(|py| {
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
        Python::with_gil(|py| {
            let fields = PyDict::new(py);
            if fields.set_item("collection", collection).is_err()
                || fields.set_item("duration_us", duration_us).is_err()
            {
                return;
            }
            self.dispatch(py, "query", &fields);
        });
    }
}
