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
        "accurate" => Some(256),
        "perfect" => Some(usize::MAX),
        _ => None,
    }
}
