//! E2E Scenario Test Suite for VelesDB README queries (VP-007).
//!
//! This integration test module validates that every query example in the README
//! actually works against the real VelesDB engine. Each submodule covers a
//! specific domain of functionality.

mod helpers;
mod hero_query;

// Wave 2 stubs — implementation in plans 04-02 through 04-06
mod cross_store;
mod match_complex;
mod match_simple;
mod metrics_and_fusion;
mod select_domain;
