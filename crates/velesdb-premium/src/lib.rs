//! # VelesDB Premium
//!
//! Premium features for VelesDB vector database.
//!
//! This crate provides advanced features available under commercial license:
//!
//! - **Hybrid Search**: Combine vector similarity with BM25 full-text search
//! - **Advanced Filtering**: Complex metadata filters with boolean logic
//! - **Encryption at Rest**: AES-256-GCM encryption for stored data
//! - **Snapshots**: Point-in-time backups and restoration
//!
//! ## License
//!
//! This crate is available under a commercial license.
//! Contact sales@velesdb.io for pricing information.

#![warn(missing_docs)]
#![warn(clippy::all)]

pub mod encryption;
pub mod filtering;
pub mod hybrid_search;
pub mod snapshots;

/// Premium feature flag check.
///
/// Returns `true` if premium features are enabled via license key.
pub fn is_premium_enabled() -> bool {
    std::env::var("VELESDB_LICENSE_KEY").is_ok()
}

/// Validates a VelesDB premium license key.
///
/// # Arguments
///
/// * `key` - The license key to validate
///
/// # Returns
///
/// `true` if the license key is valid, `false` otherwise.
pub fn validate_license(key: &str) -> bool {
    // TODO: Implement actual license validation
    // This is a placeholder for the licensing system
    !key.is_empty() && key.starts_with("VELES-")
}
