//! Binary wire formats shared across the server, CLI, and SDKs.
//!
//! These are pure byte (de)serialisers with no storage or persistence
//! dependency, so they compile on every target (including `wasm32`).

pub mod stable_hash;
pub mod vrb1;

#[cfg(test)]
mod stable_hash_property_tests;

#[cfg(test)]
mod stable_hash_tests;

// Canonical cross-engine ID hashing now lives in `wire::stable_hash`. Re-exported
// here so the historical `wire::hash_edge_id` path keeps working unchanged.
pub use stable_hash::{hash_edge_id, hash_id};
