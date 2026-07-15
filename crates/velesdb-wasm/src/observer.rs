//! `WasmObserver` — wasm-local read-path control-plane hook (audit F-5.4, #1392).
//!
//! # Why a wasm-dedicated trait (not core's `DatabaseObserver`)
//!
//! `velesdb-core`'s [`DatabaseObserver`](velesdb_core::DatabaseObserver) is
//! bound by `Send + Sync` because the core `Database` is a multi-threaded
//! server component. The WASM store graph is single-threaded and built on
//! `Rc<RefCell<VectorStore>>` (see [`crate::database::DatabaseInner`]), which
//! is neither `Send` nor `Sync`. A `Send + Sync` observer therefore cannot be
//! wired into the WASM store, and requiring one would be a lie about the
//! runtime. This module mirrors the core *contract* (borrowed context, an
//! `AccessDecision` return, allow-by-default, zero cost when absent) with a
//! trait that is deliberately **non-`Send`, non-`Sync`, and synchronous** —
//! there is no async on the read hot path.
//!
//! # Contract (replicated from core)
//!
//! - `on_query_request` has a default `Allow` implementation, so an observer
//!   overriding nothing behaves exactly as no observer at all.
//! - Implementations MUST NOT panic.
//! - Access denial flows through [`WasmAccessDecision::Deny`], **not** through
//!   an out-of-band error channel: `Deny` carries the message surfaced to the
//!   caller (empty result set + error string), `Allow` executes unmodified.
//! - When no observer is registered the gate is a single `Option` check and an
//!   early return — no context is built, nothing is allocated (zero overhead
//!   on the hot path, matching the core contract).

use std::rc::Rc;

/// The read operation being gated (wasm-local mirror of core's
/// `QueryOperationKind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmQueryOperationKind {
    /// Dense vector similarity search (`WasmCollectionHandle::search`).
    VectorSearch,
    /// Relational-style VelesQL `SELECT` (incl. JOIN / aggregation / FUSION /
    /// `NEAR` / `similarity()` / set operations — they all funnel through
    /// `velesql_select::execute`).
    Select,
    /// Graph traversal: VelesQL `MATCH` and `SELECT EDGES`.
    GraphTraversal,
}

/// Read-time context handed to the wasm read-path hook.
///
/// A borrowed view over the resolved query — no allocation on the fast path.
/// `principal` and `tenant_hint` are opaque, caller-supplied strings that the
/// gate never interprets; they exist so a future JS-registered observer can
/// carry identity/tenant hints through untouched (mirrors core's context).
#[derive(Debug, Clone)]
pub struct WasmQueryAccessContext<'a> {
    /// Target collection (or graph) name. Best-effort for `MATCH`, where the
    /// graph is inferred from the pattern; empty string when unresolved.
    pub collection: &'a str,
    /// Which read path is executing.
    pub operation: WasmQueryOperationKind,
    /// Opaque caller-supplied principal hint, forwarded untouched.
    pub principal: Option<&'a str>,
    /// Opaque caller-supplied tenant hint, forwarded untouched.
    pub tenant_hint: Option<&'a str>,
}

/// The control-plane decision returned by the read-path hook.
///
/// Kept intentionally small for WASM: `Allow` executes the read unmodified,
/// `Deny(msg)` aborts with `msg` and zero results. Scope narrowing
/// (`AllowWithScope` in core) is deliberately **not** replicated here yet —
/// see the module-level follow-up note; adding it later is additive.
pub enum WasmAccessDecision {
    /// Execute the read unmodified. Default decision.
    Allow,
    /// Abort the read and surface this message without producing results.
    Deny(String),
}

/// Wasm-local, single-threaded read-path observer port.
///
/// Deliberately **not** `Send`/`Sync` (see module docs). Register an
/// implementation via
/// [`WasmDatabase::register_observer`](crate::WasmDatabase::register_observer);
/// every read routed through the database (`search`, VelesQL `SELECT` /
/// `MATCH` / `SELECT EDGES`) consults it before touching the store.
pub trait WasmObserver {
    /// Called immediately before a read executes. Returns [`WasmAccessDecision`].
    ///
    /// The default allows every read unmodified, so an observer that overrides
    /// nothing is equivalent to no observer.
    fn on_query_request(&self, ctx: &WasmQueryAccessContext<'_>) -> WasmAccessDecision {
        let _ = ctx;
        WasmAccessDecision::Allow
    }
}

/// Read-path gate shared by every governed read.
///
/// Zero-overhead when `observer` is `None`: a single `Option` discriminant
/// check and an early return — the context struct is not even constructed.
/// Only when an observer is present do we build the borrowed context and
/// dispatch. `Deny` is mapped to an `Err(String)` the caller renders as an
/// empty result plus an error at the FFI boundary.
///
/// # Errors
///
/// Returns `Err(msg)` when the observer denies the read; `msg` is the
/// observer-supplied denial reason.
#[inline]
pub(crate) fn gate(
    observer: Option<&Rc<dyn WasmObserver>>,
    collection: &str,
    operation: WasmQueryOperationKind,
) -> Result<(), String> {
    // Fast path: no observer → nothing to build, nothing to check.
    let Some(obs) = observer else {
        return Ok(());
    };
    let ctx = WasmQueryAccessContext {
        collection,
        operation,
        principal: None,
        tenant_hint: None,
    };
    match obs.on_query_request(&ctx) {
        WasmAccessDecision::Allow => Ok(()),
        WasmAccessDecision::Deny(msg) => Err(msg),
    }
}

#[cfg(test)]
#[path = "observer_tests.rs"]
mod tests;
