//! Control-plane decision types for the read-path observer hook.
//!
//! These types form the *seam* handed to a [`DatabaseObserver`] implementation
//! on the query/read path. Core defines the vocabulary (which operation is
//! running, what an access decision looks like) but never the enforcing
//! policy — that lives behind the port as a premium observer implementation.
//!
//! All public types are `#[non_exhaustive]` so future additions (new operation
//! kinds, new decision variants, new scope fields) never break downstream
//! implementers' `match` arms or struct literals (Requirement 3.3).
//!
//! Core references no premium crate, type, or symbol here (Requirement 3.4).

use crate::velesql::Condition;

/// The read operation being gated.
///
/// Additive-only (`#[non_exhaustive]`) so introducing new operation kinds never
/// breaks a premium observer's `match` arms.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOperationKind {
    /// Dense vector similarity search.
    VectorSearch,
    /// Full-text / BM25 search.
    TextSearch,
    /// Hybrid (dense + sparse/text) fused search.
    HybridSearch,
    /// Graph traversal (`VelesQL` MATCH).
    GraphTraversal,
    /// Relational-style `VelesQL` SELECT (incl. JOIN / aggregation).
    Select,
}

/// Read-time context handed to the read-path hook.
///
/// A borrowed view over the resolved query: it carries exactly what an
/// access-control decision needs and nothing premium-specific
/// (Requirement 1.7, 3.4). Borrowing means no allocation on the fast path.
///
/// `principal` and `tenant_hint` are opaque, caller-supplied strings; core
/// never interprets their meaning — it only forwards them to the observer.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct QueryAccessContext<'a> {
    /// Target collection name.
    pub collection: &'a str,
    /// Which read path is executing.
    pub operation: QueryOperationKind,
    /// Opaque caller-supplied principal hint (e.g. user id / api-key id),
    /// passed through untouched for the observer to interpret.
    pub principal: Option<&'a str>,
    /// Opaque caller-supplied tenant hint, passed through untouched.
    pub tenant_hint: Option<&'a str>,
}

/// Optional narrowing the observer asks core to apply to a read.
///
/// Reuses the existing [`velesql::Condition`](crate::velesql::Condition)
/// AST language for row/collection narrowing — no parallel filter type is
/// introduced. Using the `VelesQL` condition (rather than the lower-level
/// [`filter::Condition`](crate::filter::Condition)) lets `apply_scope`
/// AND-compose the constraint directly into a query's WHERE clause, which is
/// itself a `velesql::Condition`.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct AccessScope {
    /// Opaque tenant scoping hint. Core records/forwards it for audit and
    /// adapter-level routing; row/collection narrowing that core *enforces*
    /// is expressed through `filter` below (kept policy-free in core).
    pub tenant: Option<String>,
    /// A `VelesQL` filter condition AND-composed into the query's WHERE
    /// clause before execution. Reuses the existing `velesql::Condition`
    /// language — no parallel filter type is introduced.
    pub filter: Option<Condition>,
}

/// The control-plane decision returned by the read-path hook (Requirement 1.3).
#[non_exhaustive]
#[derive(Debug)]
pub enum AccessDecision {
    /// Execute the query unmodified (Requirement 1.6). Default decision.
    Allow,
    /// Abort the query and return this error without producing results
    /// (Requirement 1.4).
    Deny(crate::Error),
    /// Execute with the given scope AND-composed into the filter pipeline
    /// (Requirement 1.5).
    AllowWithScope(AccessScope),
}
