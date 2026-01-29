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
        // Only disable if explicitly set to a truthy value (not just existing)
        if let Ok(val) = std::env::var("VELESDB_NO_UPDATE_CHECK") {
            let val_lower = val.to_lowercase();
            // Truthy values: "1", "true", "yes", "on", or any non-empty non-falsy value
            if val_lower != "0" && val_lower != "false" && val_lower != "no" && val_lower != "off" {
                return false;
            }
        }

        // Check positive form: VELESDB_UPDATE_CHECK
        if let Ok(val) = std::env::var("VELESDB_UPDATE_CHECK") {
            let val_lower = val.to_lowercase();
            return val_lower != "0"
                && val_lower != "false"
                && val_lower != "no"
                && val_lower != "off";
        }

        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial(env)]
    fn test_env_var_disables_update_check() {
        std::env::set_var("VELESDB_NO_UPDATE_CHECK", "1");

        let config = UpdateCheckConfig::default();
        assert!(!config.is_enabled());

        std::env::remove_var("VELESDB_NO_UPDATE_CHECK");
    }

    #[test]
    #[serial(env)]
    fn test_env_var_overrides_config() {
        std::env::set_var("VELESDB_NO_UPDATE_CHECK", "1");

        let config = UpdateCheckConfig {
            enabled: true, // Config says yes
            ..Default::default()
        };

        assert!(!config.is_enabled()); // But env says no

        std::env::remove_var("VELESDB_NO_UPDATE_CHECK");
    }

    #[test]
    #[serial(env)]
    fn test_config_disabled() {
        std::env::remove_var("VELESDB_NO_UPDATE_CHECK");
        std::env::remove_var("VELESDB_UPDATE_CHECK");

        let config = UpdateCheckConfig {
            enabled: false,
            ..Default::default()
        };

        assert!(!config.is_enabled());
    }

    #[test]
    #[serial(env)]
    fn test_default_enabled() {
        std::env::remove_var("VELESDB_NO_UPDATE_CHECK");
        std::env::remove_var("VELESDB_UPDATE_CHECK");

        let config = UpdateCheckConfig::default();
        assert!(config.is_enabled());
    }

    #[test]
    fn test_default_endpoint() {
        let config = UpdateCheckConfig::default();
        assert_eq!(config.endpoint, "https://velesdb.com/api/check");
    }

    #[test]
    fn test_default_timeout() {
        let config = UpdateCheckConfig::default();
        assert_eq!(config.timeout_ms, 2000);
    }
}
