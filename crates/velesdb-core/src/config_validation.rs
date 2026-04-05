//! `VelesConfig` validation logic.
//!
//! Extracted from `config.rs` to reduce NLOC below the 500 threshold.

use crate::config::{ConfigError, VelesConfig};

impl VelesConfig {
    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration value is invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_search()?;
        self.validate_hnsw()?;
        self.validate_limits()?;
        self.validate_server()?;
        self.validate_storage()?;
        self.validate_logging()
    }

    fn validate_search(&self) -> Result<(), ConfigError> {
        if let Some(ef) = self.search.ef_search {
            if !(16..=4096).contains(&ef) {
                return Err(ConfigError::InvalidValue {
                    key: "search.ef_search".to_string(),
                    message: format!("value {ef} is out of range [16, 4096]"),
                });
            }
        }

        if self.search.max_results == 0 || self.search.max_results > 10000 {
            return Err(ConfigError::InvalidValue {
                key: "search.max_results".to_string(),
                message: format!(
                    "value {} is out of range [1, 10000]",
                    self.search.max_results
                ),
            });
        }
        Ok(())
    }

    fn validate_hnsw(&self) -> Result<(), ConfigError> {
        if let Some(m) = self.hnsw.m {
            if !(4..=128).contains(&m) {
                return Err(ConfigError::InvalidValue {
                    key: "hnsw.m".to_string(),
                    message: format!("value {m} is out of range [4, 128]"),
                });
            }
        }

        if let Some(ef) = self.hnsw.ef_construction {
            if !(100..=2000).contains(&ef) {
                return Err(ConfigError::InvalidValue {
                    key: "hnsw.ef_construction".to_string(),
                    message: format!("value {ef} is out of range [100, 2000]"),
                });
            }
        }
        Ok(())
    }

    fn validate_limits(&self) -> Result<(), ConfigError> {
        if self.limits.max_dimensions == 0 || self.limits.max_dimensions > 65536 {
            return Err(ConfigError::InvalidValue {
                key: "limits.max_dimensions".to_string(),
                message: format!(
                    "value {} is out of range [1, 65536]",
                    self.limits.max_dimensions
                ),
            });
        }
        Ok(())
    }

    fn validate_server(&self) -> Result<(), ConfigError> {
        if self.server.port < 1024 {
            return Err(ConfigError::InvalidValue {
                key: "server.port".to_string(),
                message: format!("value {} must be >= 1024", self.server.port),
            });
        }
        Ok(())
    }

    fn validate_storage(&self) -> Result<(), ConfigError> {
        let valid_modes = ["mmap", "memory"];
        if !valid_modes.contains(&self.storage.storage_mode.as_str()) {
            return Err(ConfigError::InvalidValue {
                key: "storage.storage_mode".to_string(),
                message: format!(
                    "value '{}' is invalid, expected one of: {:?}",
                    self.storage.storage_mode, valid_modes
                ),
            });
        }
        Ok(())
    }

    fn validate_logging(&self) -> Result<(), ConfigError> {
        let valid_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_levels.contains(&self.logging.level.as_str()) {
            return Err(ConfigError::InvalidValue {
                key: "logging.level".to_string(),
                message: format!(
                    "value '{}' is invalid, expected one of: {:?}",
                    self.logging.level, valid_levels
                ),
            });
        }
        Ok(())
    }
}
