//! Update Check Module
//!
//! Checks for a newer version at start-up and, for a long-running server,
//! once a day after that. The check also tells the project how many installs
//! exist and for how long each one stays in use.
//!
//! # Privacy
//!
//! - The payload is a fixed set of fields: version, OS, architecture,
//!   edition, the calling binary, a CI flag, and the SHA-256 of a random
//!   per-install id. Nothing is derived from the machine, the user, or the
//!   data directory.
//! - The id is generated once and stored in the per-user configuration
//!   directory; an install that opted out never creates it.
//! - The full description, including what the receiving server stores, is
//!   `docs/guides/CONFIGURATION.md`.
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

mod ci;
mod config;
mod instance_id;

#[cfg(feature = "update-check")]
mod check;

pub use config::UpdateCheckConfig;

#[cfg(feature = "update-check")]
pub use check::{
    check_for_updates_with, spawn_update_check_with, spawn_update_heartbeat, UpdateCheckContext,
};

#[cfg(test)]
mod tests;
