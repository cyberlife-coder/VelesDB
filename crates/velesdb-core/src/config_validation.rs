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
/// Hard ceiling for `storage.mmap_cache_mb` (1 TiB). `0` is rejected: a
/// zero-byte mmap cache is never a meaningful configuration.
const MMAP_CACHE_MB_CAP: usize = 1_048_576;
/// Hard ceiling for `server.workers`. `0` means "auto" (derive from CPU
/// count), so it is allowed; any positive value is capped to a sane ceiling.
const WORKERS_CAP: usize = 4_096;

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

        // A zero-byte mmap cache is meaningless; cap the upper bound so an
        // out-of-range value cannot drive an absurd reservation.
        range_check_capacity(
            "storage.mmap_cache_mb",
            self.storage.mmap_cache_mb,
            MMAP_CACHE_MB_CAP,
        )?;
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
