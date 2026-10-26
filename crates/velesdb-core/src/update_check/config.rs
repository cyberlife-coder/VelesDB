//! Update Check Configuration (US-002)
//!
//! Provides configuration options for the update check feature.

use serde::Deserialize;
use std::path::Path;

/// The config file read when none was given.
const DEFAULT_CONFIG_FILE: &str = "velesdb.toml";

/// Configuration for update check feature.
///
/// # Priority Order
///
/// 1. Environment variable `VELESDB_NO_UPDATE_CHECK=1` (highest)
/// 2. Configuration file `[update_check]` section
/// 3. Default (enabled)
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCheckConfig {
    /// Enable update check (default: true)
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Update check endpoint URL
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// Timeout in milliseconds (default: 2000)
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_endpoint() -> String {
    "https://velesdb.com/api/check".to_string()
}

fn default_timeout_ms() -> u64 {
    2000
}

impl Default for UpdateCheckConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            endpoint: default_endpoint(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

impl UpdateCheckConfig {
    /// Reads the `[update_check]` table of the configuration file.
    ///
    /// `path` is the file passed with `--config`/`VELESDB_CONFIG`; without it
    /// `./velesdb.toml` is read when it exists. Only a missing `./velesdb.toml`
    /// (or a document without the table) gives the defaults; a `path` that was
    /// given and does not exist does not. A file that
    /// exists but cannot be read or parsed switches the check off and logs a
    /// warning: the section exists to express an opt-out, the CLI spawns the
    /// check before any other code reads the file, and a file that does not
    /// parse may well have held `enabled = false`.
    #[must_use]
    pub fn from_config_file(path: Option<&Path>) -> Self {
        match path {
            Some(explicit) => Self::load(explicit, true),
            None => Self::load(Path::new(DEFAULT_CONFIG_FILE), false),
        }
    }

    /// Reads `path`. A missing file gives the defaults only when it is the
    /// implicit one: an `explicit` path that does not exist is a mistake, and
    /// like any unreadable file it switches the check off.
    fn load(path: &Path, explicit: bool) -> Self {
        match std::fs::read_to_string(path) {
            Ok(document) => Self::from_toml_str(&document),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !explicit => Self::default(),
            Err(error) => Self::fail_closed(&error),
        }
    }

    /// Parses the `[update_check]` table out of a TOML document: the defaults
    /// when the table is absent, the check switched off when the document or
    /// the table cannot be read.
    fn from_toml_str(document: &str) -> Self {
        let root = match toml::from_str::<toml::Table>(document) {
            Ok(root) => root,
            Err(error) => return Self::fail_closed(&error),
        };
        match root.get("update_check") {
            None => Self::default(),
            Some(table) => table
                .clone()
                .try_into()
                .unwrap_or_else(|error| Self::fail_closed(&error)),
        }
    }

    /// The check switched off, with the reason in the log.
    fn fail_closed(error: &dyn std::fmt::Display) -> Self {
        tracing::warn!(
            %error,
            "the configuration file's [update_check] cannot be read, so the update check is disabled until it is fixed"
        );
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Check if update check is enabled.
    ///
    /// Environment variables take precedence over configuration file:
    /// - `VELESDB_NO_UPDATE_CHECK=1` or `true` → disabled
    /// - `VELESDB_UPDATE_CHECK=0` or `false` → disabled
    /// - `VELESDB_UPDATE_CHECK=1` or `true` → enabled
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.is_enabled_with(|key| std::env::var(key).ok())
    }

    /// [`is_enabled`](Self::is_enabled) reading the environment through
    /// `lookup`, so a test passes a table instead of mutating the process
    /// environment.
    pub(super) fn is_enabled_with(&self, lookup: impl Fn(&str) -> Option<String>) -> bool {
        // Negative form: VELESDB_NO_UPDATE_CHECK
        if lookup("VELESDB_NO_UPDATE_CHECK").is_some_and(|val| is_truthy(&val)) {
            return false;
        }

        // Positive form: VELESDB_UPDATE_CHECK
        if let Some(val) = lookup("VELESDB_UPDATE_CHECK") {
            return is_truthy(&val);
        }

        self.enabled
    }
}

/// `true` for the words that switch a boolean environment flag off: `0`,
/// `false`, `no`, `off`, in any case. The one list both the opt-out
/// variables and the CI detection read.
pub(super) fn is_falsy(val: &str) -> bool {
    matches!(val.to_lowercase().as_str(), "0" | "false" | "no" | "off")
}

/// Returns `true` if the value is a truthy string (not "0", "false", "no", "off").
fn is_truthy(val: &str) -> bool {
    !is_falsy(val)
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
