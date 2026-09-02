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
    value::{Uncased, UncasedStr},
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
///
/// **Reserved — parsed and validated, not yet applied.** `[limits]` and
/// `[hnsw]` reach the engine; this section does not.
/// [`VelesConfig::validate`] warns when it deviates from its defaults so a
/// config cannot silently promise behavior the engine does not deliver.
/// Wiring is tracked in issue #2087 — `query_timeout_ms` in particular needs
/// a query timeout the engine does not have, which is a feature of its own.
/// Per-query runtime overrides (`WITH (ef_search = N)`) are a separate,
/// working mechanism.
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

/// HNSW index configuration section — the deployment-wide default for the
/// graph topology of every index the engine builds.
///
/// **Applied at collection creation** (issue #2087). `m` and
/// `ef_construction` take effect through
/// [`HnswParams::from_config`](crate::index::hnsw::HnswParams::from_config),
/// under this precedence chain:
///
/// ```text
/// per-collection creation argument  >  [hnsw] section  >  HnswParams::auto(dimension)
/// ```
///
/// The values are **creation-time**: they are persisted into the collection's
/// own config and fix the graph topology, so editing this section afterwards
/// affects new collections only. Re-tuning an existing one means rebuilding
/// its index (`auto_reindex`), not reloading a file.
///
/// Per-query runtime overrides (`WITH (ef_search = N)`) are a separate,
/// working mechanism on a different axis: `ef_search` sizes the candidate pool
/// of one query; nothing here does.
///
/// `max_layers` is the exception and is still inert:
/// [`VelesConfig::validate`] warns when it is set. The HNSW layer count is
/// drawn per node by the level generator and no engine path caps it, so
/// honouring the knob is a feature, not a wiring.
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
    ///
    /// **Reserved — parsed and validated, not applied.** See the type-level
    /// note above.
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
    ///
    /// **Reserved — parsed and validated, not yet applied.** `[limits]`
    /// and `[hnsw]` reach the engine; this section does not.
    /// `VelesConfig::validate` warns when it deviates from its defaults.
    /// Wiring is tracked in issue #2087 (`data_dir` in particular conflicts
    /// with the path passed to `Database::open` and may go through
    /// deprecation instead).
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
/// **Deprecated — parsed and ignored.** Setting `enabled = true` changes
/// nothing: no group commit occurs, and every write keeps its own durability
/// barrier (the batch APIs already amortize to one barrier per call).
/// [`VelesConfig::validate`] logs a warning when the flag is set so a config
/// cannot promise behavior the engine does not deliver.
///
/// Issue #2078 resolved to retire this rather than wire it: the `WalBatcher`
/// it configured acknowledged a write before its bytes were durable, so it was
/// a write coalescer and not a group-commit protocol, and its `commit_delay_us`
/// was read by nothing — not even the batcher. The module is deleted. This
/// struct and the `[wal_batch]` table stay only so existing TOML files keep
/// loading; both go at the next major, which is the Rust API break.
///
/// When wired, group commit would batch multiple concurrent writes into a
/// single `sync_all()` call, amortizing the fsync cost across the batch.
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
            .merge(Self::env_provider());

        Self::finish(&figment)
    }

    /// Every top-level table of this struct, as a `VELESDB_*` variable would
    /// name it. Ordered longest-first so that a section whose name prefixes
    /// another cannot claim its variables (none do today; the ordering keeps
    /// that true for whatever is added next).
    ///
    /// Wider than [`Self::ENGINE_SECTIONS`] on purpose: `server` and
    /// `logging` are fields here too, and a variable naming one must resolve
    /// to that field rather than fall through as an unprefixed key.
    const ENV_SECTIONS: &'static [&'static str] = &[
        "quantization",
        "wal_batch",
        "logging",
        "storage",
        "limits",
        "search",
        "server",
        "hnsw",
    ];

    /// Maps one `VELESDB_`-stripped variable name onto the config path it
    /// addresses, splitting **only** at the boundary of a known section.
    ///
    /// `VELESDB_HNSW_EF_CONSTRUCTION` must reach `hnsw.ef_construction`, not
    /// `hnsw.ef.construction`. Figment's `split("_")` treats every underscore
    /// as a nesting separator, which no field carrying an underscore in its
    /// name survives — and that is most of them (`max_collections`,
    /// `ef_construction`, `query_timeout_ms`, ...). Splitting at the section
    /// boundary and nowhere else is what the documented names actually mean.
    ///
    /// A name whose first token is not a section passes through lowercased and
    /// unsplit, so `VELESDB_CONFIG`, `VELESDB_NO_UPDATE_CHECK` and the
    /// server's own `VELESDB_HOST` / `VELESDB_PORT` keep matching nothing
    /// here, exactly as they do today.
    ///
    /// Issue #2185: before this, the provider also carried `lowercase(false)`,
    /// which left the key uppercase and made even the single-token
    /// `VELESDB_HNSW_M` miss `hnsw.m`. Between the two defects, no documented
    /// engine variable reached its field.
    pub(crate) fn env_key_to_config_path(key: &UncasedStr) -> Uncased<'_> {
        let lowered = key.as_str().to_ascii_lowercase();
        for section in Self::ENV_SECTIONS {
            if let Some(field) = lowered
                .strip_prefix(section)
                .and_then(|rest| rest.strip_prefix('_'))
            {
                if !field.is_empty() {
                    return Uncased::from_owned(format!("{section}.{field}"));
                }
            }
        }
        Uncased::from_owned(lowered)
    }

    /// The `VELESDB_*` environment layer, built once so both loaders resolve
    /// variable names identically — the same single-mapping-point discipline
    /// `HnswParams::from_config` follows for the `[hnsw]` table.
    fn env_provider() -> Env {
        Env::prefixed("VELESDB_").map(Self::env_key_to_config_path)
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

        Self::finish(&figment)
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
            .merge(Self::env_provider());

        Self::finish(&figment)
    }

    /// Extracts a [`Self`] from an assembled [`Figment`] and validates it.
    /// Shared tail of `load_from_path`, `from_toml`, and
    /// `load_from_path_engine_only`, which differ only in how `figment` is
    /// assembled.
    fn finish(figment: &Figment) -> Result<Self, ConfigError> {
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
    #[deprecated(
        since = "5.2.0",
        note = "never read by the engine — [search] is not applied (issue #2087); \
                query-time WITH (ef_search = N) is the working override"
    )]
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
#[path = "shared_toml_tests.rs"]
mod shared_toml_tests;
