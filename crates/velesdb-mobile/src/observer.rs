//! `MobileObserver` — mobile read-path control-plane hook (audit F-5.4, #1392).
//!
//! # Governance parity on mobile
//!
//! The core read gate (`Database::open_with_observer` +
//! [`Database::gated_search`](velesdb_core::Database::gated_search) /
//! [`Database::authorize_read`](velesdb_core::Database::authorize_read)) is
//! already wired on `server` and `python`, and notify-only on `tauri`, but was
//! historically **absent** on mobile: [`crate::VelesDatabase`] opened via
//! `Database::open` and every read went through a *detached*
//! [`VectorCollection`](velesdb_core::VectorCollection) leaf that has no
//! observer reference. This module restores parity.
//!
//! Unlike the WASM sibling — whose single-threaded `Rc<RefCell<…>>` store
//! cannot satisfy core's `Send + Sync` observer bound and therefore mirrors the
//! contract with a wasm-local trait — mobile's [`crate::VelesDatabase`] is a
//! `Send + Sync` UniFFI object. It can (and does) wire the **real** core
//! [`DatabaseObserver`](velesdb_core::DatabaseObserver) seam directly, so a
//! denying observer actually fails the read closed and an
//! `AllowWithScope` observer narrows results — this is a genuine gate, not
//! notify-only.
//!
//! # Foreign (Kotlin / Swift) observers
//!
//! [`MobileObserver`] is a UniFFI **foreign-implementable** trait
//! (`with_foreign`): a Kotlin or Swift class can implement it and register it
//! through
//! [`VelesDatabase::open_with_observer`](crate::VelesDatabase::open_with_observer).
//! [`ForeignObserver`] adapts such an instance to core's `DatabaseObserver` so
//! every governed read consults it before touching the store.
//!
//! # Contract (inherited from core)
//!
//! - [`MobileObserver::on_query_request`] defaults to [`MobileAccessDecision::Allow`],
//!   so an observer overriding nothing behaves exactly as no observer at all.
//! - Implementations MUST NOT panic.
//! - Denial flows through [`MobileAccessDecision::Deny`], **not** an error
//!   channel: `Deny` carries the message surfaced to the caller (the read
//!   returns that error and zero results), `Allow` executes unmodified.
//! - With no observer registered the gate is a single `Option` check (the
//!   zero-overhead contract of the core gate).
//!
//! # Follow-up
//!
//! Scope narrowing (`AccessDecision::AllowWithScope`) is honoured end-to-end for
//! observers wired at the Rust level, but is **not** yet expressible from the
//! foreign `MobileAccessDecision` enum (which carries only `Allow` / `Deny`),
//! mirroring the WASM decision surface. Surfacing a foreign scope filter is an
//! additive follow-up; adding a variant to `MobileAccessDecision` is
//! non-breaking.

use std::sync::Arc;

use velesdb_core::{
    AccessDecision as CoreAccessDecision, DatabaseObserver, Error as CoreError,
    QueryAccessContext as CoreQueryAccessContext, QueryOperationKind as CoreQueryOperationKind,
};

/// The read operation being gated (UniFFI mirror of core's `QueryOperationKind`).
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum MobileQueryOperationKind {
    /// Dense vector similarity search.
    VectorSearch,
    /// Full-text / BM25 search.
    TextSearch,
    /// Hybrid (dense + text/sparse) fused search.
    HybridSearch,
    /// Graph traversal (`VelesQL` MATCH).
    GraphTraversal,
    /// Relational-style `VelesQL` SELECT (incl. JOIN / aggregation).
    Select,
}

impl From<CoreQueryOperationKind> for MobileQueryOperationKind {
    fn from(kind: CoreQueryOperationKind) -> Self {
        match kind {
            CoreQueryOperationKind::VectorSearch => Self::VectorSearch,
            CoreQueryOperationKind::TextSearch => Self::TextSearch,
            CoreQueryOperationKind::HybridSearch => Self::HybridSearch,
            CoreQueryOperationKind::GraphTraversal => Self::GraphTraversal,
            CoreQueryOperationKind::Select => Self::Select,
            // `QueryOperationKind` is `#[non_exhaustive]`; a future kind is
            // reported to foreign observers as the generic relational read so
            // the callback still fires and the gate still runs (advisory hint).
            _ => Self::Select,
        }
    }
}

/// Read-time context handed to a foreign [`MobileObserver`].
///
/// Owned (not borrowed) because it crosses the UniFFI boundary. `principal` and
/// `tenant_hint` are opaque, caller-supplied identity/tenant hints forwarded
/// untouched — the gate never interprets them (they are only meaningful when a
/// trusted embedder forwards a verified identity: the local-SDK trust boundary).
#[derive(uniffi::Record, Clone, Debug)]
pub struct MobileQueryContext {
    /// Target collection name.
    pub collection: String,
    /// Which read path is executing.
    pub operation: MobileQueryOperationKind,
    /// Opaque caller-supplied principal hint, forwarded untouched.
    pub principal: Option<String>,
    /// Opaque caller-supplied tenant hint, forwarded untouched.
    pub tenant_hint: Option<String>,
}

/// The control-plane decision returned by a foreign [`MobileObserver`].
///
/// Kept intentionally small for the FFI boundary: `Allow` executes the read
/// unmodified, `Deny { reason }` aborts with `reason` and zero results. Scope
/// narrowing (`AllowWithScope` in core) is deliberately **not** replicated at
/// the foreign boundary yet — see the module-level follow-up note; adding a
/// variant later is additive.
#[derive(uniffi::Enum, Clone, Debug)]
pub enum MobileAccessDecision {
    /// Execute the read unmodified. Default decision.
    Allow,
    /// Abort the read and surface `reason` without producing results.
    Deny {
        /// Human-readable denial reason surfaced to the caller.
        reason: String,
    },
}

/// Foreign-implementable read-path observer (UniFFI callback interface).
///
/// A Kotlin/Swift class implements this trait and registers it via
/// [`VelesDatabase::open_with_observer`](crate::VelesDatabase::open_with_observer).
/// Every governed read routed through the database (dense / text / hybrid /
/// sparse / multi-query search, `VelesQL` `SELECT` / `MATCH`) consults it before
/// touching the store.
#[uniffi::export(with_foreign)]
pub trait MobileObserver: Send + Sync {
    /// Called immediately before a read executes. Returns a
    /// [`MobileAccessDecision`].
    ///
    /// Implementations MUST NOT panic. Denial is expressed through
    /// [`MobileAccessDecision::Deny`], never by throwing.
    fn on_query_request(&self, context: MobileQueryContext) -> MobileAccessDecision;
}

/// Adapts a foreign [`MobileObserver`] to core's [`DatabaseObserver`] so it can
/// be injected through
/// [`Database::open_with_observer`](velesdb_core::Database::open_with_observer).
///
/// Only the read-path hook (`on_query_request`) is bridged; the lifecycle hooks
/// keep their no-op defaults (mobile has no event stream to forward them to).
pub(crate) struct ForeignObserver {
    inner: Arc<dyn MobileObserver>,
}

impl ForeignObserver {
    /// Wraps a foreign observer as a core-compatible `DatabaseObserver`.
    pub(crate) fn new(inner: Arc<dyn MobileObserver>) -> Self {
        Self { inner }
    }
}

impl DatabaseObserver for ForeignObserver {
    fn on_query_request(
        &self,
        ctx: &CoreQueryAccessContext,
    ) -> velesdb_core::Result<CoreAccessDecision> {
        let context = MobileQueryContext {
            collection: ctx.collection.to_string(),
            operation: ctx.operation.into(),
            principal: ctx.principal.map(str::to_string),
            tenant_hint: ctx.tenant_hint.map(str::to_string),
        };
        // Denial is a decision value, not an internal-failure `Err`: map the
        // foreign `Deny` to `AccessDecision::Deny` (mirrors `PyObserver`).
        Ok(match self.inner.on_query_request(context) {
            MobileAccessDecision::Allow => CoreAccessDecision::Allow,
            MobileAccessDecision::Deny { reason } => CoreAccessDecision::Deny(CoreError::Query(
                format!("read denied by observer: {reason}"),
            )),
        })
    }
}
