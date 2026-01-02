//! `VelesDB` Configuration Module
//!
//! Provides configuration file support via `velesdb.toml`, environment variables,
//! and runtime overrides.
//!
//! # Priority (highest to lowest)
//!
//! 1. Runtime overrides (API, REPL)
//! 2. Environment variables (`VELESDB_*`)
//! 3. Configuration file (`velesdb.toml`)
//! 4. Default values

use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// Configuration errors.
#[derive(Error, Debug)]
pub enum ConfigError {
    /// Failed to parse configuration file.
    #[error("Failed to parse configuration: {0}")]
    ParseError(String),

    /// Invalid configuration value.
    #[error("Invalid configuration value for '{key}': {message}")]
    InvalidValue {
        /// Configuration key that failed validation.
        key: String,
        /// Validation error message.
        message: String,
    },

    /// Configuration file not found.
    #[error("Configuration file not found: {0}")]
    FileNotFound(String),

    /// IO error.
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Search mode presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Fast search with `ef_search=64`, ~90% recall.
    Fast,
    /// Balanced search with `ef_search=128`, ~98% recall (default).
    #[default]
    Balanced,
    /// Accurate search with `ef_search=256`, ~99% recall.
    Accurate,
    /// High recall search with `ef_search=1024`, ~99.7% recall.
    HighRecall,
    /// Perfect recall with bruteforce, 100% guaranteed.
    Perfect,
}

impl SearchMode {
    /// Returns the `ef_search` value for this mode.
    #[must_use]
    pub fn ef_search(&self) -> usize {
        match self {
            Self::Fast => 64,
            Self::Balanced => 128,
            Self::Accurate => 256,
            Self::HighRecall => 1024,
            Self::Perfect => usize::MAX, // Signals bruteforce
        }
    }
}

/// Search configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// Default search mode.
    pub default_mode: SearchMode,
    /// Override `ef_search` (if set, overrides mode).
    pub ef_search: Option<usize>,
    /// Maximum results per query.
    pub max_results: usize,
    /// Query timeout in milliseconds.
    pub query_timeout_ms: u64,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_mode: SearchMode::Balanced,
            ef_search: None,
            max_results: 1000,
            query_timeout_ms: 30000,
        }
    }
}

/// HNSW index configuration section.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HnswConfig {
    /// Number of connections per node (M parameter).
    /// `None` = auto based on dimension.
    pub m: Option<usize>,
    /// Size of the candidate pool during construction.
    /// `None` = auto based on dimension.
    pub ef_construction: Option<usize>,
    /// Maximum number of layers (0 = auto).
    pub max_layers: usize,
}

/// Storage configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// Data directory path.
    pub data_dir: String,
    /// Storage mode: "mmap" or "memory".
    pub storage_mode: String,
    /// Mmap cache size in megabytes.
    pub mmap_cache_mb: usize,
    /// Vector alignment in bytes.
    pub vector_alignment: usize,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            data_dir: "./velesdb_data".to_string(),
            storage_mode: "mmap".to_string(),
            mmap_cache_mb: 1024,
            vector_alignment: 64,
        }
    }
}

/// Limits configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    /// Maximum vector dimensions.
    pub max_dimensions: usize,
    /// Maximum vectors per collection.
    pub max_vectors_per_collection: usize,
    /// Maximum number of collections.
    pub max_collections: usize,
    /// Maximum payload size in bytes.
    pub max_payload_size: usize,
    /// Maximum vectors for perfect mode (bruteforce).
    pub max_perfect_mode_vectors: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_dimensions: 4096,
            max_vectors_per_collection: 100_000_000,
            max_collections: 1000,
            max_payload_size: 1_048_576, // 1 MB
            max_perfect_mode_vectors: 500_000,
        }
    }
}

/// Server configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Host address.
    pub host: String,
    /// Port number.
    pub port: u16,
    /// Number of worker threads (0 = auto).
    pub workers: usize,
    /// Maximum HTTP body size in bytes.
    pub max_body_size: usize,
    /// Enable CORS.
    pub cors_enabled: bool,
    /// CORS allowed origins.
    pub cors_origins: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8080,
            workers: 0,                 // Auto
            max_body_size: 104_857_600, // 100 MB
            cors_enabled: false,
            cors_origins: vec!["*".to_string()],
        }
    }
}

/// Logging configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// Log level: error, warn, info, debug, trace.
    pub level: String,
    /// Log format: text or json.
    pub format: String,
    /// Log file path (empty = stdout).
    pub file: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "text".to_string(),
            file: String::new(),
        }
    }
}

/// Quantization configuration section.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QuantizationConfig {
    /// Default quantization type: none, sq8, binary.
    pub default_type: String,
    /// Enable reranking after quantized search.
    pub rerank_enabled: bool,
    /// Reranking multiplier for candidates.
    pub rerank_multiplier: usize,
}

impl Default for QuantizationConfig {
    fn default() -> Self {
        Self {
            default_type: "none".to_string(),
            rerank_enabled: true,
            rerank_multiplier: 2,
        }
    }
}

/// Main `VelesDB` configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VelesConfig {
    /// Search configuration.
    pub search: SearchConfig,
    /// HNSW index configuration.
    pub hnsw: HnswConfig,
    /// Storage configuration.
    pub storage: StorageConfig,
    /// Limits configuration.
    pub limits: LimitsConfig,
    /// Server configuration.
    pub server: ServerConfig,
    /// Logging configuration.
    pub logging: LoggingConfig,
    /// Quantization configuration.
    pub quantization: QuantizationConfig,
}

impl VelesConfig {
    /// Loads configuration from default sources.
    ///
    /// Priority: defaults < file < environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration parsing fails.
    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from_path("velesdb.toml")
    }

    /// Loads configuration from a specific file path.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the configuration file.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration parsing fails.
    pub fn load_from_path<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let figment = Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::file(path.as_ref()))
            .merge(Env::prefixed("VELESDB_").split("_").lowercase(false));

        figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Creates a configuration from a TOML string.
    ///
    /// # Arguments
    ///
    /// * `toml_str` - TOML configuration string.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing fails.
    pub fn from_toml(toml_str: &str) -> Result<Self, ConfigError> {
        let figment = Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::string(toml_str));

        figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Validates the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if any configuration value is invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate search config
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

        // Validate HNSW config
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

        // Validate limits
        if self.limits.max_dimensions == 0 || self.limits.max_dimensions > 65536 {
            return Err(ConfigError::InvalidValue {
                key: "limits.max_dimensions".to_string(),
                message: format!(
                    "value {} is out of range [1, 65536]",
                    self.limits.max_dimensions
                ),
            });
        }

        // Validate server config
        if self.server.port < 1024 {
            return Err(ConfigError::InvalidValue {
                key: "server.port".to_string(),
                message: format!("value {} must be >= 1024", self.server.port),
            });
        }

        // Validate storage mode
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

        // Validate logging level
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

    /// Returns the effective `ef_search` value.
    ///
    /// Uses explicit `ef_search` if set, otherwise derives from search mode.
    #[must_use]
    pub fn effective_ef_search(&self) -> usize {
        self.search
            .ef_search
            .unwrap_or_else(|| self.search.default_mode.ef_search())
    }

    /// Serializes the configuration to TOML.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|e| ConfigError::ParseError(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // SearchMode tests
    // ========================================================================

    #[test]
    fn test_search_mode_ef_search_values() {
        // Arrange & Act & Assert
        assert_eq!(SearchMode::Fast.ef_search(), 64);
        assert_eq!(SearchMode::Balanced.ef_search(), 128);
        assert_eq!(SearchMode::Accurate.ef_search(), 256);
        assert_eq!(SearchMode::HighRecall.ef_search(), 1024);
        assert_eq!(SearchMode::Perfect.ef_search(), usize::MAX);
    }

    #[test]
    fn test_search_mode_default_is_balanced() {
        // Arrange & Act
        let mode = SearchMode::default();

        // Assert
        assert_eq!(mode, SearchMode::Balanced);
    }

    #[test]
    fn test_search_mode_serialization() {
        // Arrange
        let mode = SearchMode::HighRecall;

        // Act
        let json = serde_json::to_string(&mode).expect("serialize");
        let deserialized: SearchMode = serde_json::from_str(&json).expect("deserialize");

        // Assert
        assert_eq!(json, "\"high_recall\"");
        assert_eq!(deserialized, mode);
    }

    // ========================================================================
    // VelesConfig default tests
    // ========================================================================

    #[test]
    fn test_config_default_values() {
        // Arrange & Act
        let config = VelesConfig::default();

        // Assert
        assert_eq!(config.search.default_mode, SearchMode::Balanced);
        assert_eq!(config.search.max_results, 1000);
        assert_eq!(config.search.query_timeout_ms, 30000);
        assert!(config.search.ef_search.is_none());
        assert_eq!(config.server.port, 8080);
        assert_eq!(config.storage.storage_mode, "mmap");
        assert_eq!(config.logging.level, "info");
    }

    #[test]
    fn test_config_effective_ef_search_from_mode() {
        // Arrange
        let config = VelesConfig::default();

        // Act
        let ef = config.effective_ef_search();

        // Assert
        assert_eq!(ef, 128); // Balanced mode
    }

    #[test]
    fn test_config_effective_ef_search_override() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.ef_search = Some(512);

        // Act
        let ef = config.effective_ef_search();

        // Assert
        assert_eq!(ef, 512);
    }

    // ========================================================================
    // TOML parsing tests
    // ========================================================================

    #[test]
    fn test_config_from_toml_minimal() {
        // Arrange
        let toml = r#"
[search]
default_mode = "fast"
"#;

        // Act
        let config = VelesConfig::from_toml(toml).expect("parse");

        // Assert
        assert_eq!(config.search.default_mode, SearchMode::Fast);
        // Other values should be defaults
        assert_eq!(config.server.port, 8080);
    }

    #[test]
    fn test_config_from_toml_full() {
        // Arrange
        let toml = r#"
[search]
default_mode = "high_recall"
ef_search = 512
max_results = 500
query_timeout_ms = 60000

[hnsw]
m = 48
ef_construction = 600

[storage]
data_dir = "/var/lib/velesdb"
storage_mode = "mmap"
mmap_cache_mb = 2048

[limits]
max_dimensions = 2048
max_perfect_mode_vectors = 100000

[server]
host = "0.0.0.0"
port = 9090
workers = 8

[logging]
level = "debug"
format = "json"
"#;

        // Act
        let config = VelesConfig::from_toml(toml).expect("parse");

        // Assert
        assert_eq!(config.search.default_mode, SearchMode::HighRecall);
        assert_eq!(config.search.ef_search, Some(512));
        assert_eq!(config.search.max_results, 500);
        assert_eq!(config.hnsw.m, Some(48));
        assert_eq!(config.hnsw.ef_construction, Some(600));
        assert_eq!(config.storage.data_dir, "/var/lib/velesdb");
        assert_eq!(config.storage.mmap_cache_mb, 2048);
        assert_eq!(config.limits.max_dimensions, 2048);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 9090);
        assert_eq!(config.server.workers, 8);
        assert_eq!(config.logging.level, "debug");
        assert_eq!(config.logging.format, "json");
    }

    #[test]
    fn test_config_from_toml_invalid_mode() {
        // Arrange
        let toml = r#"
[search]
default_mode = "ultra_fast"
"#;

        // Act
        let result = VelesConfig::from_toml(toml);

        // Assert
        assert!(result.is_err());
    }

    // ========================================================================
    // Validation tests
    // ========================================================================

    #[test]
    fn test_config_validate_success() {
        // Arrange
        let config = VelesConfig::default();

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn test_config_validate_ef_search_too_low() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.ef_search = Some(10);

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("search.ef_search"));
    }

    #[test]
    fn test_config_validate_ef_search_too_high() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.ef_search = Some(5000);

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn test_config_validate_invalid_storage_mode() {
        // Arrange
        let mut config = VelesConfig::default();
        config.storage.storage_mode = "disk".to_string();

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("storage.storage_mode"));
    }

    #[test]
    fn test_config_validate_invalid_log_level() {
        // Arrange
        let mut config = VelesConfig::default();
        config.logging.level = "verbose".to_string();

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("logging.level"));
    }

    #[test]
    fn test_config_validate_port_too_low() {
        // Arrange
        let mut config = VelesConfig::default();
        config.server.port = 80;

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("server.port"));
    }

    #[test]
    fn test_config_validate_hnsw_m_out_of_range() {
        // Arrange
        let mut config = VelesConfig::default();
        config.hnsw.m = Some(2);

        // Act
        let result = config.validate();

        // Assert
        assert!(result.is_err());
    }

    // ========================================================================
    // Serialization tests
    // ========================================================================

    #[test]
    fn test_config_to_toml() {
        // Arrange
        let config = VelesConfig::default();

        // Act
        let toml_str = config.to_toml().expect("serialize");

        // Assert
        assert!(toml_str.contains("[search]"));
        assert!(toml_str.contains("default_mode"));
        assert!(toml_str.contains("[server]"));
        assert!(toml_str.contains("port = 8080"));
    }

    #[test]
    fn test_config_roundtrip() {
        // Arrange
        let mut config = VelesConfig::default();
        config.search.default_mode = SearchMode::Accurate;
        config.search.ef_search = Some(300);
        config.server.port = 9000;

        // Act
        let toml_str = config.to_toml().expect("serialize");
        let parsed = VelesConfig::from_toml(&toml_str).expect("parse");

        // Assert
        assert_eq!(parsed.search.default_mode, SearchMode::Accurate);
        assert_eq!(parsed.search.ef_search, Some(300));
        assert_eq!(parsed.server.port, 9000);
    }
}
