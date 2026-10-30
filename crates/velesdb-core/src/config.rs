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
    /// Fast search with `ef_search=96`: 97.4% recall@10 in `recall_benchmark`
    /// (10K random 128-D points).
    Fast,
    /// Balanced search with `ef_search=160`: 99.8% recall@10 in
    /// `recall_benchmark` (default).
    #[default]
    Balanced,
    /// Accurate search with `ef_search=512`: 100% recall@10 in
    /// `recall_benchmark`, 0.98 on SIFT1M's 1M (`docs/BENCHMARKS.md`).
    Accurate,
    /// The engine's highest-recall mode, and the one value of this enum that no
    /// `ef_search` number can express.
    ///
    /// Per query it maps, through `SearchMode::quality()`, to
    /// `SearchQuality::Perfect`: an exhaustive scan that leaves the graph,
    /// the exact top-k, ties aside, at O(n) cost, refused on a collection larger
    /// than `limits.max_perfect_mode_vectors`. As a GLOBAL default in `[search]`
    /// it is applied as `accurate`, with a warning at load, because one search
    /// path cannot enforce that cap.
    ///
    /// This doc said the opposite until #2246 — that `SearchQuality::Perfect`
    /// "stays on the graph" at `ef_search = 4096`. `try_search_special_quality`
    /// routes it to `search_brute_force`. #2239 corrected that claim in
    /// `params.rs` and missed this copy of it.
    Perfect,
}

impl SearchMode {
    /// Returns the `ef_search` value for this mode.
    ///
    /// **Lossy for [`Self::Perfect`], which is why wiring must not use it.**
    /// `usize::MAX` was written as a bruteforce sentinel and nothing reads it as
    /// one: the only conversion available, `ef_to_quality`, turns it into
    /// `SearchQuality::Custom(usize::MAX)` — an uncapped graph traversal that is
    /// neither exhaustive nor covered by `limits.max_perfect_mode_vectors`.
    /// A config asking for `perfect` would get a silently downgraded mode and a
    /// bypassed guard rail. Use [`Self::quality`] instead (#2238).
    #[must_use]
    pub fn ef_search(&self) -> usize {
        match self {
            Self::Fast => 96,
            Self::Balanced => 160,
            Self::Accurate => 512,
            Self::Perfect => usize::MAX,
        }
    }

    /// Returns the [`SearchQuality`](crate::SearchQuality) this mode means.
    ///
    /// The lossless counterpart to [`Self::ef_search`]: every mode maps onto a
    /// named quality, including `Perfect`, which no `ef_search` number can
    /// express because the engine special-cases the variant rather than the
    /// value. This is the conversion the `[search]` wiring of issue #2087 must
    /// use.
    ///
    /// Gated on `persistence`, like `SearchQuality`'s own export: without that
    /// feature there is no HNSW index for a quality to describe. `SearchMode`
    /// itself stays available, because a config file parses on every build.
    #[cfg(feature = "persistence")]
    #[must_use]
    pub fn quality(&self) -> crate::SearchQuality {
        match self {
            Self::Fast => crate::SearchQuality::Fast,
            Self::Balanced => crate::SearchQuality::Balanced,
            Self::Accurate => crate::SearchQuality::Accurate,
            Self::Perfect => crate::SearchQuality::Perfect,
        }
    }
}

/// Search configuration section.
///
/// **Half applied.** `default_mode` and `ef_search` reach the engine since
/// #2087: an unqualified `search()` resolves through
/// [`SearchConfig::resolved_quality`], with `ef_search` winning over
/// `default_mode` when both are set.
///
/// `max_results` and `query_timeout_ms` are still inert and still warned about
/// by [`VelesConfig::validate`], field by field. They are not oversights:
/// clamping a caller's `LIMIT` silently is a footgun and erroring is a breaking
/// change under a *default* config, while a query timeout is a feature the
/// engine does not have. Both need a decision, not plumbing.
///
/// Per-query runtime overrides (`WITH (ef_search = N)`) are a separate,
/// working mechanism and continue to win over anything here.
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

impl SearchConfig {
    /// The [`SearchQuality`](crate::SearchQuality) an unqualified `search()`
    /// resolves to under this section.
    ///
    /// `ef_search` wins over `default_mode` when set, which is what the field's
    /// own doc has always promised ("if set, overrides mode") and what nothing
    /// used to honour, the section being inert.
    ///
    /// Goes through [`SearchMode::quality`] rather than
    /// [`SearchMode::ef_search`] on purpose: the latter cannot express
    /// `Perfect`, and routing through it would hand a `perfect` config an
    /// uncapped traversal with `max_perfect_mode_vectors` bypassed (#2238).
    ///
    /// `Perfect` itself is applied as `Accurate` here. A per-query `Perfect` is
    /// an exhaustive scan capped by `enforce_perfect_mode_limit`; as a global
    /// default it could not be applied at all: a filtered search's bitmap
    /// pre-filter never reads the configured quality — it traverses the graph at
    /// its own ef — so a `perfect` default would scan on some queries and
    /// traverse on others. `VelesConfig::validate` warns when this downgrade
    /// happens rather than failing the load.
    ///
    /// Available only with the `persistence` feature, the only build that
    /// exports [`SearchQuality`](crate::SearchQuality).
    #[cfg(feature = "persistence")]
    #[must_use]
    pub fn resolved_quality(&self) -> crate::SearchQuality {
        self.ef_search.map_or_else(
            || match self.default_mode {
                // Applied as `Accurate`, and warned about at load by
                // `validate()`: a filtered search's bitmap pre-filter never
                // reads this quality, so a global `Perfect` would scan on some
                // queries and traverse on others (see the doc above).
                SearchMode::Perfect => crate::SearchQuality::Accurate,
                mode => mode.quality(),
            },
            crate::SearchQuality::Custom,
        )
    }
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
    /// **Reserved.** `storage_mode` is parsed and validated but not applied,
    /// pending its own decision (distinct from `quantization::StorageMode`;
    /// whether the engine has a memory-only mode to select is not
    /// established). [`crate::config::VelesConfig::validate`] warns when it
    /// is set away from its default.
    ///
    /// `data_dir`, `mmap_cache_mb` and `vector_alignment` were removed in
    /// 7.0.0 (#2087): none of them had an engine counterpart, and
    /// `data_dir` conflicted irreducibly with the path passed to
    /// `Database::open`. A file that still sets one loads, and the loader
    /// names it in a warning.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(default)]
    pub struct StorageConfig {
        /// Storage mode: `"mmap"` or `"memory"`.
        pub storage_mode: String,
    }

    impl Default for StorageConfig {
        fn default() -> Self {
            Self {
                storage_mode: "mmap".to_string(),
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
    /// The tables a release removed are not listed: they are no field of this
    /// struct. [`Self::env_key_to_config_path`] still splits at them, from
    /// `config_removed`, so their variables are named by the removal warning
    /// instead of flattening into a key nothing reports.
    ///
    /// Wider than [`Self::ENGINE_SECTIONS`] on purpose: `server` and
    /// `logging` are fields here too, and a variable naming one must resolve
    /// to that field rather than fall through as an unprefixed key.
    const ENV_SECTIONS: &'static [&'static str] = &[
        "quantization",
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
        let sections = Self::ENV_SECTIONS
            .iter()
            .copied()
            .chain(crate::config_removed::removed_sections());
        for section in sections {
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
    const ENGINE_SECTIONS: &'static [&'static str] =
        &["search", "hnsw", "storage", "limits", "quantization"];

    /// Drops every top-level TOML table not in [`Self::ENGINE_SECTIONS`],
    /// except a table a release removed: that one is kept so the loader can
    /// name it in its removal warning, and extraction then ignores it like
    /// any key no field claims.
    fn filter_to_engine_sections(raw: &str) -> Result<String, ConfigError> {
        let mut doc: toml::Value =
            toml::from_str(raw).map_err(|e| ConfigError::ParseError(e.to_string()))?;
        if let Some(table) = doc.as_table_mut() {
            table.retain(|k, _| {
                Self::ENGINE_SECTIONS.contains(&k)
                    || crate::config_removed::removed_sections().any(|removed| removed == k)
            });
        }
        toml::to_string(&doc).map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Loads configuration from a specific file path, considering **only**
    /// the engine sections (`[search]`/`[hnsw]`/`[storage]`/`[limits]`/
    /// `[quantization]`) and silently dropping any other top-level table
    /// before parsing — notably `[server]` and `[logging]`. A table a release
    /// removed is the one exception: it is not applied either, but it is
    /// named in a warning rather than dropped without a word.
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
    /// `[limits] max_collections` from the file. The environment is filtered
    /// the same way the file is, by [`Self::is_engine_path`]: a variable for
    /// a non-engine section (`VELESDB_SERVER_*`, `VELESDB_LOGGING_*`) is not
    /// read here at all. It used to be — `server` and `logging` are fields of
    /// this struct, so such a variable reached them and was validated against
    /// rules for values this loader never applies: `VELESDB_SERVER_PORT=80`
    /// stopped `velesdb-server` (which binds `VELESDB_PORT`) with
    /// `server.port must be >= 1024`. Those sections belong to the hosting
    /// shell, which reads and validates them once, itself.
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
            .merge(Self::env_provider().filter(|path| Self::is_engine_path(path.as_str())));

        Self::finish(&figment)
    }

    /// Whether a mapped config path (`section.field`, as
    /// [`Self::env_key_to_config_path`] produces it) belongs to an engine
    /// section, or to a section a release removed — kept, as in
    /// [`Self::filter_to_engine_sections`], so its removal warning still
    /// fires. Everything else, including a name that maps to no section at
    /// all, is the hosting shell's.
    pub(crate) fn is_engine_path(path: &str) -> bool {
        path.split_once('.').is_some_and(|(section, _)| {
            Self::ENGINE_SECTIONS.contains(&section)
                || crate::config_removed::removed_sections().any(|removed| removed == section)
        })
    }

    /// Extracts a [`Self`] from an assembled [`Figment`] and validates it.
    /// Shared tail of `load_from_path`, `from_toml`, and
    /// `load_from_path_engine_only`, which differ only in how `figment` is
    /// assembled.
    ///
    /// The removal warning is raised here, from the merged sources, because
    /// the extracted struct no longer has anywhere to hold a removed key.
    fn finish(figment: &Figment) -> Result<Self, ConfigError> {
        let config: Self = figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;
        crate::config_removed::warn_removed_keys(figment);
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
