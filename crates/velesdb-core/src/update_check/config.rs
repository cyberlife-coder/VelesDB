//! Update Check Configuration (US-002)
//!
//! Provides configuration options for the update check feature.

use serde::Deserialize;
use std::path::Path;

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
    /// `./velesdb.toml` is read when it exists. A missing, unreadable or
    /// malformed file, or a missing table, gives the defaults: the update
    /// check never fails start-up, and the engine's own loader is the one
    /// that reports a bad file.
    #[must_use]
    pub fn from_config_file(path: Option<&Path>) -> Self {
        let default_path = Path::new("velesdb.toml");
        let path = path.unwrap_or(default_path);
        std::fs::read_to_string(path)
            .map(|document| Self::from_toml_str(&document))
            .unwrap_or_default()
    }

    /// Parses the `[update_check]` table out of a TOML document.
    fn from_toml_str(document: &str) -> Self {
        #[derive(Deserialize)]
        struct Document {
            update_check: Option<UpdateCheckConfig>,
        }

        toml::from_str::<Document>(document)
            .ok()
            .and_then(|parsed| parsed.update_check)
            .unwrap_or_default()
    }

    /// Check if update check is enabled.
    ///
    /// Environment variables take precedence over configuration file:
    /// - `VELESDB_NO_UPDATE_CHECK=1` or `true` → disabled
    /// - `VELESDB_UPDATE_CHECK=0` or `false` → disabled
    /// - `VELESDB_UPDATE_CHECK=1` or `true` → enabled
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        // Check negative form: VELESDB_NO_UPDATE_CHECK
        if let Ok(val) = std::env::var("VELESDB_NO_UPDATE_CHECK") {
            if is_truthy(&val) {
                return false;
            }
        }

        // Check positive form: VELESDB_UPDATE_CHECK
        if let Ok(val) = std::env::var("VELESDB_UPDATE_CHECK") {
            return is_truthy(&val);
        }

        self.enabled
    }
}

/// Returns `true` if the value is a truthy string (not "0", "false", "no", "off").
fn is_truthy(val: &str) -> bool {
    !matches!(val.to_lowercase().as_str(), "0" | "false" | "no" | "off")
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
