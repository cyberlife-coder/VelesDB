//! `VelesConfig` validation logic.
//!
//! Extracted from `config.rs` to reduce NLOC below the 500 threshold.

use crate::config::{ConfigError, VelesConfig};

// ---------------------------------------------------------------------------
// Upper-bound caps for capacity/size limits.
//
// These caps reject absurd values that would silently invite resource
// exhaustion or integer-overflow surprises downstream, while staying well
// above every realistic deployment (and above the crate defaults so the
// default config validates through the loaders). `0` is rejected for
// capacities/sizes because a zero capacity is never a meaningful config.
// ---------------------------------------------------------------------------

/// Hard ceiling for `limits.max_vectors_per_collection`.
///
/// On 64-bit targets this is 10 billion. On 32-bit / WASM targets `usize`
/// is only 32 bits (max ≈ 4.29 billion), so the literal is capped at
/// 4 billion to prevent a compile-time integer-overflow error.
#[cfg(target_pointer_width = "64")]
const MAX_VECTORS_PER_COLLECTION_CAP: usize = 10_000_000_000;
#[cfg(not(target_pointer_width = "64"))]
const MAX_VECTORS_PER_COLLECTION_CAP: usize = 4_000_000_000;
/// Hard ceiling for `limits.max_collections` (1 million).
const MAX_COLLECTIONS_CAP: usize = 1_000_000;
/// Hard ceiling for `limits.max_payload_size` (1 GiB).
const MAX_PAYLOAD_SIZE_CAP: usize = 1_073_741_824;
/// Hard ceiling for `limits.max_perfect_mode_vectors` (100 million).
const MAX_PERFECT_MODE_VECTORS_CAP: usize = 100_000_000;
/// Hard ceiling for `search.query_timeout_ms` (24 hours). `0` means
/// "disabled". The previous 1-hour cap rejected legitimate long batch
/// timeouts; 24h is generous enough for any real query while still rejecting
/// effectively-unbounded values.
const QUERY_TIMEOUT_MS_CAP: u64 = 86_400_000;
/// Hard ceiling for `hnsw.max_layers`. `0` means "auto".
const MAX_LAYERS_CAP: usize = 64;
use crate::config::{LOG_LEVELS, WORKERS_CAP};

/// Rejects `0` and any value above `cap` for a capacity/size field.
fn range_check_capacity(key: &str, value: usize, cap: usize) -> Result<(), ConfigError> {
    if value == 0 || value > cap {
        return Err(ConfigError::InvalidValue {
            key: key.to_string(),
            message: format!("value {value} is out of range [1, {cap}]"),
        });
    }
    Ok(())
}

/// Range-checks a field where `0` is a valid sentinel (disabled / auto) but any
/// positive value must not exceed `cap`. Unlike [`range_check_capacity`], `0`
/// is accepted.
fn range_check_upper<T: PartialOrd + Copy + std::fmt::Display>(
    key: &str,
    value: T,
    cap: T,
) -> Result<(), ConfigError> {
    if value > cap {
        return Err(ConfigError::InvalidValue {
            key: key.to_string(),
            message: format!("value {value} is out of range [0, {cap}]"),
        });
    }
    Ok(())
}

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
        self.validate_logging()?;
        self.warn_inert_engine_sections();
        self.warn_perfect_global_default();
        Ok(())
    }

    /// `storage.storage_mode` and `[search]`'s `max_results` /
    /// `query_timeout_ms` are parsed and validated but not applied (issue
    /// #2087); `[search]`'s `default_mode` and `ef_search` are applied since.
    /// Warn — rather than reject — when a config sets an inert entry away from
    /// its default, so existing files keep loading while no deployment
    /// silently believes those knobs work.
    ///
    /// `[hnsw]` is no longer listed wholesale: `m` and `ef_construction` are
    /// applied at collection creation. Only `max_layers` remains inert — the
    /// layer count is drawn per node by the level generator and no engine
    /// path caps it — so the warning narrows to that single field. `[storage]`
    /// is narrowed the same way: `storage_mode` is its only field since
    /// 7.0.0 removed the three that had no engine counterpart (#2087), and
    /// it stays reported here pending its own decision. Reporting the field
    /// rather than the section is the point: a warning that fires on knobs
    /// that now work, or that are leaving rather than pending, would train
    /// readers to ignore it.
    ///
    /// Serde-value comparison instead of `PartialEq` derives: the sections
    /// carry enums and nested types, and this runs once per config load.
    fn warn_inert_engine_sections(&self) {
        let inert = self.inert_engine_entries();
        if !inert.is_empty() {
            tracing::warn!(
                // Entries are a mix of whole sections and single keys now
                // that `[hnsw]` is partly wired, hence `inert` rather than
                // the former `sections`.
                inert = inert.join(", "),
                "these config entries are parsed and validated but not \
                 applied by the engine; see issue #2087. [limits] and \
                 [hnsw]'s m / ef_construction are applied. Query-time \
                 WITH (...) overrides are unaffected."
            );
        }
    }

    /// `search.default_mode = "perfect"` is accepted, applied as `accurate` by
    /// `SearchConfig::resolved_quality`, and said so here.
    ///
    /// It was refused outright until the seven-lens review (#2246): a file
    /// v6.0.0 loaded then failed `Database::open` -- a breaking change in a
    /// minor release, against this module's own rule that existing files keep
    /// loading. The default still never resolves to `Perfect`: a filtered
    /// search's bitmap pre-filter never reads the configured quality — it
    /// traverses the graph at its own ef — so a `perfect` default would be an
    /// exhaustive scan on some queries and a graph search on others, with
    /// nothing saying which. (It used to be argued that one path could not
    /// refuse; since #2246 every path returns its error, and this is what
    /// remains.) Per query, `search_with_quality(Perfect)` stays available and
    /// capped.
    fn warn_perfect_global_default(&self) {
        if matches!(self.search.default_mode, crate::config::SearchMode::Perfect) {
            tracing::warn!(
                "search.default_mode = \"perfect\" is applied as \"accurate\": a \
                 filtered search's pre-filter never reads the configured quality, \
                 so a global exhaustive scan would apply to some queries and not \
                 others. Request Perfect per query, where it is exact and capped."
            );
        }
    }

    /// The config entries this build parses and validates but does not apply,
    /// as they would be named in a TOML file.
    ///
    /// Split out of [`Self::warn_inert_engine_sections`] so the set is
    /// assertable: a `tracing` warning is not observable from a test, and the
    /// claim this list makes — that a configured `hnsw.m` is *not* inert while
    /// `hnsw.max_layers` still is — is precisely what the #2087 wiring has to
    /// keep honest as more knobs land.
    pub(crate) fn inert_engine_entries(&self) -> Vec<&'static str> {
        fn deviates<T: serde::Serialize>(actual: &T, default: &T) -> bool {
            match (serde_json::to_value(actual), serde_json::to_value(default)) {
                (Ok(a), Ok(d)) => a != d,
                _ => false,
            }
        }

        let mut inert: Vec<&'static str> = Vec::new();
        // Field by field since #2087 wired `default_mode` and `ef_search`:
        // reporting the whole section would fire on knobs that now work, which
        // is how a warning trains its readers to ignore it.
        let search_default = crate::config::SearchConfig::default();
        if self.search.max_results != search_default.max_results {
            inert.push("search.max_results");
        }
        if self.search.query_timeout_ms != search_default.query_timeout_ms {
            inert.push("search.query_timeout_ms");
        }
        if self.hnsw.max_layers != crate::config::HnswConfig::default().max_layers {
            inert.push("hnsw.max_layers");
        }
        if self.storage.storage_mode != crate::config::server::StorageConfig::default().storage_mode
        {
            inert.push("storage.storage_mode");
        }
        if deviates(
            &self.quantization,
            &crate::config_quantization::QuantizationConfig::default(),
        ) {
            inert.push("[quantization]");
        }
        inert
    }

    fn validate_search(&self) -> Result<(), ConfigError> {
        if let Some(ef) = self.search.ef_search {
            // Same range the CLI's `\set ef_search` and every query-time
            // `WITH (ef_search = ...)` enforce (#2274) — one definition.
            crate::api_types::validate_ef_search(ef).map_err(|message| {
                ConfigError::InvalidValue {
                    key: "search.ef_search".to_string(),
                    message,
                }
            })?;
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

        // `query_timeout_ms == 0` disables the timeout (see `QueryContext`);
        // any positive value is capped to avoid effectively-unbounded queries.
        range_check_upper(
            "search.query_timeout_ms",
            self.search.query_timeout_ms,
            QUERY_TIMEOUT_MS_CAP,
        )
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

        // `max_layers == 0` means "auto" (see `HnswConfig`); a positive value
        // is capped to a sane ceiling.
        range_check_upper("hnsw.max_layers", self.hnsw.max_layers, MAX_LAYERS_CAP)
    }

    fn validate_limits(&self) -> Result<(), ConfigError> {
        let limits = &self.limits;
        range_check_capacity("limits.max_dimensions", limits.max_dimensions, 65536)?;
        range_check_capacity(
            "limits.max_vectors_per_collection",
            limits.max_vectors_per_collection,
            MAX_VECTORS_PER_COLLECTION_CAP,
        )?;
        range_check_capacity(
            "limits.max_collections",
            limits.max_collections,
            MAX_COLLECTIONS_CAP,
        )?;
        range_check_capacity(
            "limits.max_payload_size",
            limits.max_payload_size,
            MAX_PAYLOAD_SIZE_CAP,
        )?;
        range_check_capacity(
            "limits.max_perfect_mode_vectors",
            limits.max_perfect_mode_vectors,
            MAX_PERFECT_MODE_VECTORS_CAP,
        )
    }

    fn validate_server(&self) -> Result<(), ConfigError> {
        if self.server.port < 1024 {
            return Err(ConfigError::InvalidValue {
                key: "server.port".to_string(),
                message: format!("value {} must be >= 1024", self.server.port),
            });
        }

        // `workers == 0` means "auto" (derive from CPU count); a positive
        // value is capped so a typo cannot spawn an absurd thread count.
        range_check_upper("server.workers", self.server.workers, WORKERS_CAP)
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
        if !LOG_LEVELS.contains(&self.logging.level.as_str()) {
            return Err(ConfigError::InvalidValue {
                key: "logging.level".to_string(),
                message: format!(
                    "value '{}' is invalid, expected one of: {:?}",
                    self.logging.level, LOG_LEVELS
                ),
            });
        }
        Ok(())
    }
}
