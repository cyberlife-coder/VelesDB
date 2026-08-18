//! Synchronization primitives with loom support for concurrency testing.
//!
//! This module provides type aliases that switch between standard library
//! sync primitives and loom's mocked versions based on the `loom` feature flag.
//!
//! # Usage
//!
//! ```rust,no_run
//! use velesdb_core::sync::{Arc, RwLock, Mutex};
//!
//! // Works with both std and loom
//! let data = Arc::new(RwLock::new(42));
//! ```
//!
//! # Testing with Loom
//!
//! The loom tests are gated on `cfg(loom)`. The crate's `build.rs` bridges the
//! `loom` Cargo feature to that cfg, so the feature flag alone is enough — no
//! `RUSTFLAGS` needed. Both the integration models and the storage models
//! additionally require the `persistence` feature:
//!
//! ```bash
//! # Integration models (tests/loom_tests.rs):
//! cargo test -p velesdb-core --features loom,persistence --test loom_tests
//! # Storage models (src/storage/loom_tests.rs, a unit-test target):
//! cargo test -p velesdb-core --features loom,persistence --lib storage::loom
//! ```
//!
//! CI runs the same models on a schedule with `RUSTFLAGS="--cfg loom"` set
//! explicitly (`quality-deep.yml`); the build.rs makes local runs work without
//! it. Note: these validate hand-written loom *models* of the lock ordering,
//! not the production `parking_lot`/`dashmap` types directly.
//!
//! # EPIC-023: Loom Concurrency Testing

// ============================================================================
// Arc
// ============================================================================

#[cfg(loom)]
pub use loom::sync::Arc;

#[cfg(not(loom))]
pub use std::sync::Arc;

// ============================================================================
// Mutex (Note: We use parking_lot in production, but loom provides its own)
// ============================================================================

#[cfg(loom)]
pub use loom::sync::Mutex;

#[cfg(not(loom))]
pub use parking_lot::Mutex;

// ============================================================================
// RwLock
// ============================================================================

#[cfg(loom)]
pub use loom::sync::RwLock;

#[cfg(not(loom))]
pub use parking_lot::RwLock;

// ============================================================================
// Atomics
// ============================================================================

#[cfg(loom)]
pub use loom::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

#[cfg(not(loom))]
pub use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

// ============================================================================
// Thread spawning (for loom tests)
// ============================================================================

#[cfg(loom)]
pub use loom::thread;

#[cfg(not(loom))]
pub use std::thread;
