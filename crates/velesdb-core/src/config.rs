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
    /// Accurate search with `ef_search=256`, ~100% recall.
    Accurate,
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

/// Quantization configuration section (EPIC-073/US-005).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QuantizationConfig {
    /// Default quantization type: none, sq8, binary.
    pub default_type: String,
    /// Enable reranking after quantized search.
    pub rerank_enabled: bool,
    /// Reranking multiplier for candidates.
    pub rerank_multiplier: usize,
    /// Auto-enable quantization for large collections (EPIC-073/US-005).
    pub auto_quantization: bool,
    /// Threshold for auto-quantization (number of vectors).
    pub auto_quantization_threshold: usize,
}

impl Default for QuantizationConfig {
    fn default() -> Self {
        Self {
            default_type: "none".to_string(),
            rerank_enabled: true,
            rerank_multiplier: 2,
            auto_quantization: true,
            auto_quantization_threshold: 10_000,
        }
    }
}

impl QuantizationConfig {
    /// Returns whether quantization should be used based on vector count (EPIC-073/US-005).
    #[must_use]
    pub fn should_quantize(&self, vector_count: usize) -> bool {
        self.auto_quantization && vector_count >= self.auto_quantization_threshold
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
