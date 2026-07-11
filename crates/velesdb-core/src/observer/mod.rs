//! `DatabaseObserver` — extension hook for velesdb-premium.
//!
//! The core library has zero knowledge of Premium internals.
//! Premium implements this trait and injects it via
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
/// Implement this trait in `velesdb-premium` to attach RBAC, audit logging,
/// multi-tenant routing, or replication logic without modifying the core.
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
/// # Example (Premium side)
///
/// ```rust,ignore
/// use velesdb_core::{DatabaseObserver, CollectionType};
///
/// struct PremiumObserver { /* audit_log, rbac, tenant_router */ }
///
/// impl DatabaseObserver for PremiumObserver {
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

    /// Called after points are upserted into a collection.
    fn on_upsert(&self, _collection: &str, _point_count: usize) {}

    /// Called after a query is executed, with the duration in microseconds.
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
    /// graph traversal, `VelesQL` SELECT).
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
