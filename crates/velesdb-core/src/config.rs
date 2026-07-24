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
    /// Perfect recall via **exhaustive bruteforce** (`ef_search = usize::MAX`
    /// signals a full scan): every vector is scored, no HNSW graph traversal, so
    /// recall is 100% by construction at O(n) cost.
    ///
    /// Distinct from `SearchQuality::Perfect`
    /// (`crate::index::hnsw::SearchQuality`) despite the shared name:
    /// `SearchMode` picks the **engine** (bruteforce here vs. the HNSW graph),
    /// whereas `SearchQuality::Perfect` stays *on* the graph with a very high
    /// `ef_search` (`4096.max(k*100)`) — ~1.0 recall up to ~100K, ~0.9994 at 1M,
    /// at graph cost rather than a full scan. Pick `SearchMode::Perfect` only
    /// when an exact guarantee is worth the linear scan.
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
///
/// `#[non_exhaustive]`: build from [`LimitsConfig::default`] and adjust fields
/// so future limits stay backward compatible for downstream crates.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
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

        let config: Self = figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate()?;
        Ok(config)
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

        let config: Self = figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// The top-level TOML tables that belong to the *engine* — as opposed
    /// to `server` and `logging`, which are also fields on this struct but
    /// exist for standalone/embedded consumers of `VelesConfig`. A hosting
    /// shell (e.g. `velesdb-server`) that owns its own same-named
    /// `[server]` table in the same file — different shape, different
    /// meaning (HTTP bind port vs. this struct's own `server.port`) — would
    /// otherwise have that table parsed into *this* struct too and
    /// rejected by [`Self::validate`]'s rules for a value it was never
    /// meant to apply to. See [`Self::load_from_path_engine_only`].
    const ENGINE_SECTIONS: &'static [&'static str] = &[
        "search",
        "hnsw",
        "storage",
        "limits",
        "quantization",
        "wal_batch",
    ];

    /// Drops every top-level TOML table not in [`Self::ENGINE_SECTIONS`].
    fn filter_to_engine_sections(raw: &str) -> Result<String, ConfigError> {
        let mut doc: toml::Value =
            toml::from_str(raw).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        if let Some(table) = doc.as_table_mut() {
            table.retain(|k, _| Self::ENGINE_SECTIONS.contains(&k));
        }
        toml::to_string(&doc).map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Loads configuration from a specific file path, considering **only**
    /// the engine sections (`[search]`/`[hnsw]`/`[storage]`/`[limits]`/
    /// `[quantization]`/`[wal_batch]`) and silently dropping any other
    /// top-level table before parsing — notably `[server]` and `[logging]`.
    ///
    /// Use this instead of [`Self::load_from_path`] when the TOML file is
    /// **shared** with a hosting shell that owns its own `[server]`/
    /// `[auth]`/`[tls]`/`[cors]`/... sections under possibly-colliding
    /// keys — e.g. `velesdb-server --config` reads the same file for its
    /// own HTTP transport settings (`[server].port` = the bind port) *and*
    /// for this engine config. Without filtering, `[server] port = 443`
    /// (a perfectly legitimate low bind port, e.g. behind `setcap`/a
    /// privileged process) would also land in *this* struct's
    /// `server.port` and be rejected by [`Self::validate`]'s `port >=
    /// 1024` rule — a spurious failure with nothing to do with the actual
    /// value being configured.
    ///
    /// As with [`Self::load_from_path`], `VELESDB_*` environment variables
    /// are layered on top of the (filtered) file and can still override an
    /// engine value — e.g. `VELESDB_LIMITS_MAX_COLLECTIONS=5` overrides a
    /// `[limits] max_collections` from the file. Env vars for non-engine
    /// sections (`VELESDB_SERVER_*`, `VELESDB_LOGGING_*`, ...) are
    /// harmless here: they don't match any field once those sections are
    /// filtered out of the base document, so they're ignored the same way
    /// an unrecognised key always is.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, is not valid TOML, or
    /// fails validation.
    pub fn load_from_path_engine_only<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path.as_ref())?;
        let filtered = Self::filter_to_engine_sections(&raw)?;

        let figment = Figment::new()
            .merge(Serialized::defaults(Self::default()))
            .merge(Toml::string(&filtered))
            .merge(Env::prefixed("VELESDB_").split("_").lowercase(false));

        let config: Self = figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Same as [`Self::load_from_path_engine_only`] but from an in-memory
    /// TOML string, with no environment-variable layer — mirrors how
    /// [`Self::from_toml`] relates to [`Self::load_from_path`].
    ///
    /// # Errors
    ///
    /// Returns an error if `toml_str` is not valid TOML or fails
    /// validation.
    pub fn from_toml_engine_only(toml_str: &str) -> Result<Self, ConfigError> {
        let filtered = Self::filter_to_engine_sections(toml_str)?;
        Self::from_toml(&filtered)
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

#[cfg(test)]
mod shared_toml_tests {
    use super::*;

    /// Documents why `_engine_only` exists: `[server] port = 443` is a
    /// legitimate low HTTP bind port for a hosting shell (e.g.
    /// `velesdb-server` behind `setcap`), but fed through the
    /// whole-struct loader it lands in *this* crate's own `server.port`
    /// and trips `validate_server`'s `>= 1024` rule — a real, reproducible
    /// bug when a shell shares its `velesdb.toml` with `VelesConfig`
    /// as-is (not a regression test to "fix" — `load_from_path` is
    /// correct for standalone/embedded use where `[server]` truly
    /// belongs to `VelesConfig`).
    #[test]
    fn test_load_from_path_whole_struct_rejects_shell_owned_low_port() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let path = dir.path().join("velesdb.toml");
        std::fs::write(
            &path,
            "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
        )
        .expect("test: write toml");

        let err = VelesConfig::load_from_path(&path).expect_err(
            "whole-struct loader must still reject port=443 via its own server section",
        );
        assert!(
            err.to_string().contains("server.port"),
            "unexpected error: {err}"
        );
    }

    /// The actual fix: a shell-owned `[server] port = 443` no longer
    /// leaks into `VelesConfig`'s own `server` section, and the genuine
    /// engine section (`[limits]`) is still applied.
    #[test]
    fn test_load_from_path_engine_only_ignores_shell_owned_server_section() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let path = dir.path().join("velesdb.toml");
        std::fs::write(
            &path,
            "[server]\nport = 443\n\n[limits]\nmax_collections = 5\n",
        )
        .expect("test: write toml");

        let config = VelesConfig::load_from_path_engine_only(&path)
            .expect("engine-only loader must ignore the shell-owned [server] section");

        // The engine section came through.
        assert_eq!(config.limits.max_collections, 5);
        // The shell-owned [server] section did NOT — the struct's own
        // `server.port` stays at its default, proving the table was
        // dropped rather than parsed-then-happening-to-pass-validation.
        assert_eq!(config.server.port, ServerConfig::default().port);
    }

    #[test]
    fn test_from_toml_engine_only_ignores_shell_owned_server_section() {
        let config = VelesConfig::from_toml_engine_only(
            "[server]\nport = 443\n\n[limits]\nmax_collections = 7\n",
        )
        .expect("engine-only parser must ignore the shell-owned [server] section");

        assert_eq!(config.limits.max_collections, 7);
        assert_eq!(config.server.port, ServerConfig::default().port);
    }

    #[test]
    fn test_load_from_path_engine_only_still_applies_non_server_engine_sections() {
        let dir = tempfile::tempdir().expect("test: temp dir");
        let path = dir.path().join("velesdb.toml");
        std::fs::write(
            &path,
            "[hnsw]\nm = 24\n\n[wal_batch]\nenabled = true\ncommit_delay_us = 250\n",
        )
        .expect("test: write toml");

        let config = VelesConfig::load_from_path_engine_only(&path)
            .expect("engine-only loader must still apply hnsw/wal_batch");

        assert_eq!(config.hnsw.m, Some(24));
        assert!(config.wal_batch.enabled);
        assert_eq!(config.wal_batch.commit_delay_us, 250);
    }

    #[test]
    fn test_load_from_path_engine_only_missing_file_errors() {
        let missing = std::path::Path::new("/nonexistent/velesdb-issue-1549-engine-only.toml");
        assert!(VelesConfig::load_from_path_engine_only(missing).is_err());
    }

    #[test]
    fn test_from_toml_engine_only_invalid_value_still_fails_typed() {
        // max_collections = 0 is out of range — the fix must not silently
        // swallow real validation errors, only shell-owned sections.
        let err = VelesConfig::from_toml_engine_only("[limits]\nmax_collections = 0\n")
            .expect_err("out-of-range engine value must still fail");
        assert!(
            err.to_string().contains("limits.max_collections"),
            "unexpected error: {err}"
        );
    }
}
