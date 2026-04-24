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

// Re-export quantization types so existing `crate::config::Quantization*` paths work.
pub use crate::config_quantization::{QuantizationConfig, QuantizationType};

/// Configuration errors.
#[derive(Error, Debug)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum SearchMode {
    /// Fast search with `ef_search=96`, ~95% recall.
    Fast,
    /// Balanced search with `ef_search=160`, ~99.5% recall (default).
    #[default]
    Balanced,
    /// Accurate search with `ef_search=512`, ~100% recall.
    Accurate,
    /// Perfect recall with bruteforce, 100% guaranteed.
    Perfect,
}

impl SearchMode {
    /// Returns the `ef_search` value for this mode.
    #[must_use]
    pub fn ef_search(&self) -> usize {
        match self {
            Self::Fast => 96,
            Self::Balanced => 160,
            Self::Accurate => 512,
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

/// Server-layer configuration types (HTTP transport, logging, storage paths).
///
/// These types are intentionally separated from the core engine configuration
/// (`SearchConfig`, `HnswConfig`, `LimitsConfig`) to enforce layer boundaries.
/// Import via `config::server::ServerConfig` or use the crate-root re-exports.
pub mod server {
    use serde::{Deserialize, Serialize};

    /// Storage configuration section.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default)]
    pub struct StorageConfig {
        /// Data directory path.
        pub data_dir: String,
        /// Storage mode: `"mmap"` or `"memory"`.
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
                workers: 0,
                max_body_size: 104_857_600,
                cors_enabled: false,
                cors_origins: vec!["*".to_string()],
            }
        }
    }

    /// Logging configuration section.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default)]
    pub struct LoggingConfig {
        /// Log level: `error`, `warn`, `info`, `debug`, `trace`.
        pub level: String,
        /// Log format: `text` or `json`.
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
}

// Backward-compatible re-exports at module level.
pub use server::{LoggingConfig, ServerConfig, StorageConfig};

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

// ---------------------------------------------------------------------------
// WAL batch commit configuration
// ---------------------------------------------------------------------------

/// Default commit delay in microseconds for WAL group commit.
const fn default_commit_delay_us() -> u64 {
    100
}

/// Default maximum entries per WAL batch.
const fn default_max_batch_size() -> usize {
    128
}

/// Configuration for WAL group commit batching.
///
/// When enabled, multiple concurrent writes are batched into a single
/// `sync_all()` call, amortizing the fsync cost across the batch.
///
/// # Example (TOML)
///
/// ```toml
/// [wal_batch]
/// enabled = true
/// commit_delay_us = 200
/// max_batch_size = 256
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalBatchConfig {
    /// Whether group commit is enabled. Default: `false`.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum delay in microseconds before flushing a batch. Default: `100`.
    #[serde(default = "default_commit_delay_us")]
    pub commit_delay_us: u64,
    /// Maximum number of entries per batch. Default: `128`.
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
}

impl Default for WalBatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            commit_delay_us: 100,
            max_batch_size: 128,
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
    /// WAL group commit batching configuration.
    pub wal_batch: WalBatchConfig,
}

impl VelesConfig {
    /// Loads configuration from default sources.
    ///
    /// Priority: defaults < file < environment variables.
    ///
    /// # Errors
    ///
    /// Returns `ConfigError` if the configuration file is malformed or
    /// environment variables contain invalid values.
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

    // Validation is in config_validation.rs

    /// Returns the effective `ef_search` value.
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
