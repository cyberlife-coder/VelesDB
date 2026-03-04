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

use crate::collection::CollectionType;

/// Lifecycle hooks for database events.
///
/// Implement this trait in `velesdb-premium` to attach RBAC, audit logging,
/// multi-tenant routing, or replication logic without modifying the core.
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
}
