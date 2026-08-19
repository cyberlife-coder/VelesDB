//! Update Check Configuration (US-002)
//!
//! Provides configuration options for the update check feature.

use serde::Deserialize;

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
