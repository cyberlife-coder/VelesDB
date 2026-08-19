// Reason: clippy 1.90 similar_names flags idiomatic test bindings (dir/dim, ids/idx).
#![allow(clippy::similar_names)]
//! Shared test fixtures for `velesdb-core` tests.
//!
//! Centralizes collection creation, point generation, and setup patterns
//! to avoid duplication across test modules. All items are `#[cfg(test)]`
//! gated at the module level.
//!
//! # Usage
//!
//! ```rust,ignore
//! use crate::test_fixtures::fixtures::{setup_collection, make_point};
//!
//! let (_dir, col) = setup_collection(4);
//! let p = make_point(1, vec![1.0, 0.0, 0.0, 0.0]);
//! ```

#[cfg(test)]
#[path = "fixtures.rs"]
pub(crate) mod fixtures;
