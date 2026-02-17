//! Update Check Module
//!
//! Provides automatic update checking at startup to help users stay secure.
//! This is similar to VS Code, Firefox, and most modern software.
//!
//! # Privacy
//!
//! - No personal information is collected
//! - Instance hash is SHA256, not reversible
//! - IP addresses are not logged server-side
//! - Easily disabled via environment variable
//!
//! # Disabling
//!
//! ```bash
//! export VELESDB_NO_UPDATE_CHECK=1
//! ```
//!
//! Or in config file:
//! ```toml
//! [update_check]
//! enabled = false
//! ```
#![allow(clippy::doc_markdown)] // Docs intentionally reference product/arch identifiers.

mod config;
mod instance_id;

#[cfg(feature = "update-check")]
mod check;

pub use config::UpdateCheckConfig;
pub use instance_id::compute_instance_hash;

#[cfg(feature = "update-check")]
pub use check::{check_for_updates, spawn_update_check};

#[cfg(test)]
mod tests;
