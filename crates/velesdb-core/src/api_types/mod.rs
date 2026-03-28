//! Canonical request/response DTOs shared across API layers.
//!
//! This module contains data transfer objects used by both `velesdb-server`
//! (REST API) and `tauri-plugin-velesdb` (desktop IPC). Server enables the
//! `openapi` feature for `utoipa::ToSchema` derives; Tauri re-exports directly.

mod requests;
mod responses;
#[cfg(test)]
mod tests;

pub use requests::*;
pub use responses::*;

/// Canonical `VelesQL` contract version for REST responses.
pub const VELESQL_CONTRACT_VERSION: &str = "3.0.0";

// ============================================================================
// Shared default value functions
// ============================================================================

/// Default distance metric: cosine.
#[must_use]
pub fn default_metric() -> String {
    "cosine".to_string()
}

/// Default storage mode: full (no quantization).
#[must_use]
pub fn default_storage_mode() -> String {
    "full".to_string()
}

/// Default number of results to return.
#[must_use]
pub const fn default_top_k() -> usize {
    10
}

/// Default vector weight for hybrid search.
#[must_use]
pub const fn default_vector_weight() -> f32 {
    0.5
}

/// Default collection type: vector.
#[must_use]
pub fn default_collection_type() -> String {
    "vector".to_string()
}

/// Default fusion strategy: RRF.
#[must_use]
pub fn default_fusion_strategy() -> String {
    "rrf".to_string()
}

/// Default RRF k parameter.
#[must_use]
pub const fn default_rrf_k() -> u32 {
    60
}

/// Default average weight for weighted fusion.
#[must_use]
pub const fn default_avg_weight() -> f32 {
    0.5
}

/// Default max weight for weighted fusion.
#[must_use]
pub const fn default_max_weight() -> f32 {
    0.3
}

/// Default hit weight for weighted fusion.
#[must_use]
pub const fn default_hit_weight() -> f32 {
    0.2
}

/// Default index type: hash.
#[must_use]
pub fn default_index_type() -> String {
    "hash".to_string()
}

/// Convert search mode string to `ef_search` value.
#[must_use]
pub fn mode_to_ef_search(mode: &str) -> Option<usize> {
    match mode.to_lowercase().as_str() {
        "fast" => Some(64),
        "balanced" => Some(128),
        "accurate" => Some(512),
        "perfect" => Some(usize::MAX),
        _ => None,
    }
}

/// Convert search mode string to [`SearchQuality`].
///
/// Supports all named modes including `"autotune"` which adapts ef
/// automatically based on collection statistics, plus advanced modes:
/// - `"custom:<ef>"` for a custom `ef_search` value
/// - `"adaptive:<min_ef>:<max_ef>"` for two-phase adaptive search
#[cfg(feature = "persistence")]
#[must_use]
pub fn mode_to_search_quality(mode: &str) -> Option<crate::SearchQuality> {
    match mode.to_lowercase().as_str() {
        "fast" => Some(crate::SearchQuality::Fast),
        "balanced" => Some(crate::SearchQuality::Balanced),
        "accurate" => Some(crate::SearchQuality::Accurate),
        "perfect" => Some(crate::SearchQuality::Perfect),
        "autotune" | "auto_tune" | "auto" => Some(crate::SearchQuality::AutoTune),
        other => parse_advanced_quality(other),
    }
}

/// Parses advanced search quality modes: `custom:<ef>` and `adaptive:<min_ef>:<max_ef>`.
#[cfg(feature = "persistence")]
fn parse_advanced_quality(mode: &str) -> Option<crate::SearchQuality> {
    if let Some(ef_str) = mode.strip_prefix("custom:") {
        let ef = ef_str.parse::<usize>().ok()?;
        return Some(crate::SearchQuality::Custom(ef));
    }
    if let Some(params) = mode.strip_prefix("adaptive:") {
        let parts: Vec<&str> = params.split(':').collect();
        if parts.len() == 2 {
            let min_ef = parts[0].parse::<usize>().ok()?;
            let max_ef = parts[1].parse::<usize>().ok()?;
            if min_ef <= max_ef {
                return Some(crate::SearchQuality::Adaptive { min_ef, max_ef });
            }
        }
    }
    None
}
