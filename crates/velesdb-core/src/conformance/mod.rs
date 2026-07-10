//! Cross-implementation conformance harness (Requirement 7).
//!
//! This module provides frozen *golden* reference input→output vectors for the
//! invariants that MUST agree byte-for-byte across every `VelesDB`
//! implementation — core, premium, and any alternate engine. Two independent
//! implementations of the same operation are conformant iff they reproduce
//! these reference outputs exactly.
//!
//! # Why a shared harness
//!
//! Premium historically forked core's VelesQL parser, JOIN executor, and score
//! fusion. Forks drift. This harness lets premium (or any consumer) run the
//! same reference checks against *its own* functions and detect divergence
//! before it reaches production:
//!
//! ```
//! use velesdb_core::conformance;
//! use velesdb_core::hash_id;
//!
//! // A candidate implementation is conformant when the check returns no
//! // divergences.
//! let divergences = conformance::check_stable_hash(hash_id);
//! assert!(divergences.is_empty(), "stable-hash divergence: {divergences:?}");
//! ```
//!
//! # What is verified
//!
//! - [`check_stable_hash`] — the stable string→`u64` derivation
//!   ([`crate::hash_id`], FNV-1a). Requirement 7.1.
//! - [`check_rrf`] — Reciprocal Rank Fusion scoring/ordering
//!   ([`crate::fusion::FusionStrategy::RRF`]). Requirement 7.2.
//! - [`check_executor`] — a focused, well-defined shared *executor* operation:
//!   the canonical graph edge-id derivation ([`crate::hash_edge_id`]). A full
//!   VelesQL query-executor reference is intentionally out of scope for a
//!   frozen golden table (it would require a populated `Database` and would not
//!   be persistence-free); the edge-id derivation is the representative
//!   deterministic executor primitive that premium's forked graph/JOIN
//!   executor depends on. Requirement 7.3.
//!
//! Each `check_*` function returns a `Vec<`[`Divergence`]`>`; an **empty**
//! vector means full agreement, and each entry identifies exactly which case
//! diverged (Requirement 7.4). The same functions are runnable against any
//! candidate implementation by passing that implementation's function
//! (Requirement 7.5).
//!
//! The harness is persistence-free and `wasm32`-safe, so every binding crate
//! can run it.

mod executor;
mod fusion;
mod hash;

#[cfg(test)]
mod executor_tests;
#[cfg(test)]
mod property_tests;

pub use executor::{check_executor, executor_reference_vectors, ExecutorVector};
pub use fusion::{check_rrf, rrf_reference_vectors, FusionVector};
pub use hash::{check_stable_hash, hash_reference_vectors, HashVector};

/// A single reference case whose candidate output diverged from the frozen
/// golden output.
///
/// `case` identifies the diverging input, `expected` is the frozen golden
/// rendering, and `actual` is the candidate implementation's rendering. All
/// three are human-readable strings so a divergence report is
/// implementation-agnostic and easy to log in a test failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// Human-readable identifier of the diverging input case.
    pub case: String,
    /// The frozen golden output, rendered for display.
    pub expected: String,
    /// The candidate implementation's output, rendered for display.
    pub actual: String,
}
