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
//! ```bash
//! cargo +nightly test --features loom --test loom_tests
//! ```
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
