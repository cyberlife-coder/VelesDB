//! Upsert telemetry: how a collection reports its completed writes to the
//! [`DatabaseObserver`] of the database that registered it.
//!
//! Core fires `on_upsert` itself, from the storage entry points every write
//! funnels through — `Collection::upsert` (vector path), `upsert_metadata`,
//! `upsert_bulk`, `upsert_bulk_from_raw` and `store_node_payloads` — so a
//! write reaches the observer once whatever surface made it: `VelesQL` DML, a
//! REST handler, a binding, the streaming drain. Entry points that delegate to
//! another one (metadata-only `upsert`, `store_node_payload`, a graph node
//! with an embedding) report through the one they delegate to, never twice.
//!
//! A collection opened directly (`Collection::create`/`open`, not through a
//! `Database`) has no observer and reports nothing.

use std::sync::{Arc, OnceLock};

use crate::observer::DatabaseObserver;

use super::types::Collection;

/// The observer a registered collection reports to, with the name it is
/// registered under.
pub(crate) struct UpsertReporter {
    collection: String,
    observer: Arc<dyn DatabaseObserver>,
}

/// Shared by every clone of one collection, so the registry's copy and the
/// handles it hands out report to the same observer. Set once, at
/// registration: a database's observer never changes after open.
pub(crate) type UpsertReporterSlot = Arc<OnceLock<UpsertReporter>>;

impl Collection {
    /// Wires this collection's writes to `observer` under `collection`.
    ///
    /// Called by `Database` registration only. A second call is a no-op: the
    /// slot belongs to the database that admitted the collection first.
    pub(crate) fn attach_upsert_observer(
        &self,
        collection: &str,
        observer: Arc<dyn DatabaseObserver>,
    ) {
        // Ignoring the `Err` is the documented no-op on a second attach.
        let _ = self.runtime.upsert_reporter.set(UpsertReporter {
            collection: collection.to_owned(),
            observer,
        });
    }

    /// Reports a completed write of `point_count` points.
    ///
    /// Call only after the write succeeded, and with no storage lock held:
    /// the observer is foreign code. A write of zero points stored nothing
    /// and is not reported.
    pub(crate) fn report_upsert(&self, point_count: usize) {
        if point_count == 0 {
            return;
        }
        if let Some(reporter) = self.runtime.upsert_reporter.get() {
            reporter
                .observer
                .on_upsert(&reporter.collection, point_count);
        }
    }
}
