//! The option values a `CREATE COLLECTION` names by string — its metric
//! and its storage mode — read as every engine reads them: core's DDL
//! executor and the WASM executor resolve them here, so the names accepted
//! and the refusal's words are the same on both.

use crate::error::{Error, Result};

/// Resolves a metric name string to a `DistanceMetric` enum.
///
/// # Errors
///
/// [`Error::Query`] (`VELES-010`) if the metric name is unrecognized.
pub fn resolve_metric(s: &str) -> Result<crate::DistanceMetric> {
    crate::DistanceMetric::parse_alias(s).ok_or_else(|| {
        Error::Query(format!(
            "Unknown metric '{s}'. Use: {}",
            crate::distance::METRIC_ALIAS_LIST
        ))
    })
}

/// Resolves an optional storage mode string to a `StorageMode` enum.
///
/// Defaults to `StorageMode::Full` when `None` is provided.
///
/// # Errors
///
/// [`Error::Query`] (`VELES-010`) if the storage mode name is unrecognized.
pub fn resolve_storage_mode(s: Option<&str>) -> Result<crate::StorageMode> {
    let Some(name) = s else {
        return Ok(crate::StorageMode::default());
    };
    crate::StorageMode::parse_alias(name).ok_or_else(|| {
        Error::Query(format!(
            "Unknown storage mode '{name}'. Use: full, sq8, binary, pq, rabitq"
        ))
    })
}

#[cfg(test)]
#[path = "ddl_options_tests.rs"]
mod tests;
