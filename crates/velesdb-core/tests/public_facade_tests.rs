//! Sentinel tests that pin the **public Facade** of `velesdb-core` (audit-2026q2 H2).
//!
//! Wrapper crates (server, python, wasm, mobile, cli, migrate, tauri) and downstream
//! users should depend only on symbols re-exported at the crate root. If any
//! `pub use` line in `lib.rs` is removed or moved, these tests fail to compile,
//! signalling a breaking change in the public surface that must be coordinated.
//!
//! These tests verify *visibility*, not behaviour — they only assert that the type
//! can be named via the crate root. They do not exercise functionality.
//!
//! When adding a new type that wrapper crates need, add it here too.
//!
//! BDD pattern: GIVEN the crate root, WHEN a wrapper imports `velesdb_core::X`,
//! THEN compilation must succeed and the type must be the one defined in core.

#![cfg(feature = "persistence")]

#[test]
fn observability_types_are_root_exported() {
    // GIVEN-WHEN-THEN: each of these is a real path that the server's `lib.rs`
    // (and any future observability-focused wrapper) relies on. Resolving them
    // through the crate root proves the Facade is intact.
    let _ = velesdb_core::DurationHistogram::new();
    let _ = velesdb_core::OperationalMetrics::new();
    let _ = velesdb_core::TraversalMetrics::new();
    let _ = velesdb_core::QueryStats { collection: String::from("facade"), ..Default::default() };
    let _ = velesdb_core::GuardRailsMetrics::new();
}

#[test]
fn guardrail_types_are_root_exported() {
    let ql = velesdb_core::QueryLimits::default();
    assert!(
        ql.max_depth > 0,
        "QueryLimits default must bound traversal depth"
    );
    assert!(
        ql.max_cardinality > 0,
        "QueryLimits default must bound cardinality"
    );
}

#[test]
fn core_search_types_are_root_exported() {
    // These were already re-exported before the audit but are re-pinned here
    // so the sentinel catches accidental removal during future refactors.
    let _: Option<velesdb_core::HnswParams> = None;
    let _: Option<velesdb_core::SearchQuality> = None;
    let _: Option<velesdb_core::DistanceMetric> = None;
    let _: Option<velesdb_core::StorageMode> = None;
    let _: Option<velesdb_core::QuantizationConfig> = None;
}

#[test]
fn graph_types_are_root_exported() {
    let _: Option<velesdb_core::GraphEdge> = None;
    let _: Option<velesdb_core::GraphNode> = None;
    let _: Option<velesdb_core::GraphSchema> = None;
    let _: Option<velesdb_core::TraversalConfig> = None;
    let _: Option<velesdb_core::TraversalResult> = None;
}
