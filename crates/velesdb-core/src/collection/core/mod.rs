//! Core Collection implementation (Lifecycle & CRUD).
//!
//! This module provides the main implementation of Collection:
//! - Lifecycle: create, open, flush, save
//! - CRUD: upsert, get, delete

mod crud;
mod lifecycle;

// All implementations are in submodules, no re-exports needed here
// as they extend the Collection type defined in types.rs
