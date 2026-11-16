//! `DatabaseObserver` — the extension hook of an out-of-tree policy layer.
//!
//! The core library has zero knowledge of any implementation of it. A policy
//! layer implements this trait and injects it via
//! [`Database::open_with_observer`](crate::Database::open_with_observer).
//!
//! # Contract
//!
//! - All methods have default no-op implementations.
//! - Implementations MUST be `Send + Sync`.
//! - Implementations MUST NOT panic.
//! - Overhead when `observer` is `None` is a single pointer check.

pub mod context;

#[cfg(test)]
mod backward_compat_tests;

#[cfg(test)]
mod context_tests;

pub use context::{AccessDecision, AccessScope, QueryAccessContext, QueryOperationKind};

use crate::collection::CollectionType;

/// Lifecycle and control-plane hooks for database events.
///
/// Implement this trait in an out-of-tree policy layer to attach RBAC, audit
/// logging, multi-tenant routing, or replication logic without modifying the
/// core.
///
/// # Contract
///
/// - All methods have default implementations: telemetry/lifecycle hooks are
///   no-ops and gate hooks are allow-all. A consumer that supplies no observer,
///   or an observer overriding only a subset of methods, behaves exactly as it
///   did before any hook existed.
/// - Implementations MUST be `Send + Sync`.
/// - Implementations MUST NOT panic.
/// - Access denial flows through a decision value
///   ([`AccessDecision::Deny`]), **not** through the `Result` error channel:
///   the `Err` variant of a gate is reserved for internal observer failures.
///
/// Every method has a default implementation, so adding future hooks is not a
/// breaking change for downstream implementers — they keep compiling by
/// relying on the defaults (the trait-level equivalent of `#[non_exhaustive]`,
/// which Rust does not permit on traits directly).
///
/// # Example (implementation side)
///
/// ```rust,ignore
/// use velesdb_core::{DatabaseObserver, CollectionType};
///
/// struct PolicyObserver { /* audit_log, rbac, tenant_router */ }
///
/// impl DatabaseObserver for PolicyObserver {
///     fn on_collection_created(&self, name: &str, kind: &CollectionType) {
///         // self.audit_log.record(...)
///     }
/// }
/// ```
pub trait DatabaseObserver: Send + Sync {
    /// Called after a collection is successfully created.
    fn on_collection_created(&self, _name: &str, _kind: &CollectionType) {}

    /// Called after a collection is successfully deleted.
    fn on_collection_deleted(&self, _name: &str) {}

    /// Called once after each completed write to a collection the database
    /// registered, whatever surface made it, with the number of points the
    /// write carried — a batch repeating an id counts each entry, though the
    /// last one wins in storage. A failed or empty write is not reported.
    fn on_upsert(&self, _collection: &str, _point_count: usize) {}

    /// Called once after each completed search or `VelesQL` read, with the
    /// collection the read gate decided on and the read's duration in
    /// microseconds: `execute_query`, `execute_aggregate`, `gated_search` and
    /// `run_gated_read`. A `VelesQL` statement is one read, reported once with
    /// its scalar subqueries folded in, and `explain_analyze_query` reports
    /// the read it executes. A refused or failed read is not reported, and
    /// neither are the reads gated without a search (point lookups, scrolls,
    /// graph reads).
    fn on_query(&self, _collection: &str, _duration_us: u64) {}

    /// Called before a DDL statement is executed.
    ///
    /// Premium extensions can implement this to enforce RBAC policies
    /// (e.g., only admin users can CREATE/DROP collections).
    ///
    /// Returns `Ok(())` to allow the DDL operation, or `Err(Error)` to reject it.
    /// Default implementation allows all DDL operations.
    ///
    /// # Errors
    ///
    /// Implementations should return an error to reject the DDL operation.
    fn on_ddl_request(&self, operation: &str, collection_name: &str) -> crate::Result<()> {
        let _ = (operation, collection_name);
        Ok(())
    }

    /// Called before a mutating DML statement is executed.
    ///
    /// Premium extensions can implement this to enforce RBAC policies
    /// (e.g., restrict INSERT EDGE, DELETE, or DELETE EDGE to authorized users).
    ///
    /// Returns `Ok(())` to allow the DML mutation, or `Err(Error)` to reject it.
    /// Default implementation allows all DML mutations.
    ///
    /// # Errors
    ///
    /// Implementations should return an error to reject the DML mutation.
    fn on_dml_mutation_request(&self, operation: &str, collection_name: &str) -> crate::Result<()> {
        let _ = (operation, collection_name);
        Ok(())
    }

    /// Called in the core use-case layer immediately before a query/read
    /// executes, for every read path (vector search, text/BM25, hybrid,
    /// graph traversal, `VelesQL` SELECT, aggregate and `SELECT EDGES`).
    ///
    /// A `VelesQL` statement asks once per collection it reads: its target,
    /// each set-operation operand, each JOIN's secondary collection, each
    /// `MATCH` `@collection`, and each scalar subquery's collection. The
    /// consultation contract is **once per `(collection, operation)` per
    /// statement** (issue #2588): however a statement fans out internally —
    /// a self-JOIN, a set operation over one collection, a repeated
    /// `@collection`, a subquery on the outer collection, or the plan read an
    /// `EXPLAIN`/`EXPLAIN ANALYZE` makes before the row read — the first
    /// answer is replayed for the statement's lifetime, so a logging or
    /// metering observer never double-counts one logical query.
    ///
    /// A scope is AND-composed into the `WHERE` of the target and of each
    /// operand; on a read with no row filter to compose it into (a traversal,
    /// a JOIN's secondary side, `@collection` enrichment, `SELECT EDGES`) a
    /// scope filter refuses the statement with
    /// [`Error::Config`](crate::Error::Config) instead of running it unscoped.
    ///
    /// Premium extensions can implement this to enforce RBAC, tenant scoping,
    /// and row/collection filtering on the read path so that every consumer of
    /// core inherits control-plane behavior through the port.
    ///
    /// Returns an [`AccessDecision`] the core enforces before producing
    /// results: [`Allow`](AccessDecision::Allow) executes unmodified,
    /// [`Deny`](AccessDecision::Deny) aborts with the supplied error and zero
    /// results, and [`AllowWithScope`](AccessDecision::AllowWithScope) narrows
    /// the query before execution. The default allows every read unmodified,
    /// so the open path is unchanged.
    ///
    /// # Errors
    ///
    /// Implementations return `Err` only for internal failures; access denial
    /// is expressed through [`AccessDecision::Deny`], not the `Result` error.
    fn on_query_request(&self, ctx: &QueryAccessContext) -> crate::Result<AccessDecision> {
        let _ = ctx;
        Ok(AccessDecision::Allow)
    }
}
