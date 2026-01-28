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
    /// Environment variable takes precedence over configuration file.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        // Env var takes absolute precedence
        if std::env::var("VELESDB_NO_UPDATE_CHECK").is_ok() {
            return false;
        }

        // Also support the positive form
        if let Ok(val) = std::env::var("VELESDB_UPDATE_CHECK") {
            return val != "0" && val.to_lowercase() != "false";
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
