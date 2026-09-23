//! Canonical request/response DTOs shared across API layers.
//!
//! This module contains data transfer objects used by both `velesdb-server`
//! (REST API) and `tauri-plugin-velesdb` (desktop IPC). Server enables the
//! `openapi` feature for `utoipa::ToSchema` derives; Tauri re-exports directly.

mod requests;
mod responses;
mod responses_explain;
pub mod serde_id;
#[cfg(test)]
mod serde_id_collection_tests;
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
///
/// Derived from [`crate::fusion::DEFAULT_WEIGHTED_AVG_WEIGHT`] — the fusion
/// module is the single source of truth for these defaults (#1545); this
/// wrapper exists only because serde `default` needs a function path. The
/// REST surface previously froze the pre-#1545 values (0.5/0.3/0.2) here,
/// forking the default fusion behavior from every other surface.
#[must_use]
pub const fn default_avg_weight() -> f32 {
    crate::fusion::DEFAULT_WEIGHTED_AVG_WEIGHT
}

/// Default max weight for weighted fusion.
/// See [`default_avg_weight`] for the single-source rationale.
#[must_use]
pub const fn default_max_weight() -> f32 {
    crate::fusion::DEFAULT_WEIGHTED_MAX_WEIGHT
}

/// Default hit weight for weighted fusion.
/// See [`default_avg_weight`] for the single-source rationale.
#[must_use]
pub const fn default_hit_weight() -> f32 {
    crate::fusion::DEFAULT_WEIGHTED_HIT_WEIGHT
}

/// Default dense weight for relative score fusion.
#[must_use]
pub const fn default_dense_weight() -> f32 {
    0.5
}

/// Default sparse weight for relative score fusion.
#[must_use]
pub const fn default_sparse_weight() -> f32 {
    0.5
}

/// Default index type: hash.
#[must_use]
pub fn default_index_type() -> String {
    "hash".to_string()
}

/// Convert search mode string to [`crate::SearchQuality`].
///
/// Supports all named modes including `"autotune"` which adapts ef
/// automatically based on collection statistics, plus advanced modes:
/// - `"custom:<ef>"` for a custom `ef_search` value
/// - `"adaptive:<min_ef>:<max_ef>"` for two-phase adaptive search
///
/// Returns `None` for a mode it cannot read. An entry point that must refuse
/// such a mode rather than fall back uses [`parse_search_mode`].
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
///
/// `ef`, `min_ef` and `max_ef` are each checked against
/// [`validate_ef_search`]'s `[MIN_EF_SEARCH, MAX_EF_SEARCH]` range — the same
/// bound the dedicated `ef_search` option enforces — so this spelling of the
/// option cannot reach the search path with an unbounded `ef` (#2275).
#[cfg(feature = "persistence")]
fn parse_advanced_quality(mode: &str) -> Option<crate::SearchQuality> {
    if let Some(ef_str) = mode.strip_prefix("custom:") {
        let ef = ef_str.parse::<usize>().ok()?;
        validate_ef_search(ef).ok()?;
        return Some(crate::SearchQuality::Custom(ef));
    }
    if let Some(params) = mode.strip_prefix("adaptive:") {
        let parts: Vec<&str> = params.split(':').collect();
        if parts.len() == 2 {
            let min_ef = parts[0].parse::<usize>().ok()?;
            let max_ef = parts[1].parse::<usize>().ok()?;
            if min_ef <= max_ef {
                validate_ef_search(min_ef).ok()?;
                validate_ef_search(max_ef).ok()?;
                return Some(crate::SearchQuality::Adaptive { min_ef, max_ef });
            }
        }
    }
    None
}

/// The accepted search mode forms, as every mode error names them.
#[cfg(feature = "persistence")]
pub(crate) const SEARCH_MODE_FORMS: &str = concat!(
    "Valid values: 'fast', 'balanced', 'accurate', 'perfect', ",
    "'autotune' (aliases: 'auto_tune', 'auto'), 'custom:<ef>', ",
    "'adaptive:<min_ef>:<max_ef>' (min_ef <= max_ef, both in the ef_search range)"
);

/// Parses a search mode string into a [`crate::SearchQuality`], or an error
/// naming the accepted forms when it cannot be parsed.
///
/// Delegates to [`mode_to_search_quality`] for the parsing itself; unlike
/// that function, an unparseable mode is a distinct `Err` here rather than a
/// `None` a caller might mistake for "no mode given". Use this at entry
/// points where the caller should reject a typo instead of silently falling
/// back to the default quality (#2267).
///
/// # Errors
///
/// Returns a message naming the accepted forms when `mode` matches none of
/// them (an unknown name, or `custom:`/`adaptive:` with a malformed or
/// out-of-order argument).
#[cfg(feature = "persistence")]
pub fn parse_search_mode(mode: &str) -> Result<crate::SearchQuality, String> {
    mode_to_search_quality(mode)
        .ok_or_else(|| format!("Unknown search mode '{mode}'. {SEARCH_MODE_FORMS}"))
}

/// Minimum accepted `ef_search`, per `docs/VELESQL_SPEC.md`.
pub const MIN_EF_SEARCH: usize = 16;

/// Maximum accepted `ef_search`, per `docs/VELESQL_SPEC.md`.
pub const MAX_EF_SEARCH: usize = 4096;

/// The message every `ef_search` rejection reports, `shown` being the value
/// the caller gave: a `usize` for [`validate_ef_search`], the original
/// `i64` for [`parse_with_ef_search`] so a negative value prints as itself
/// rather than the `usize` it failed to become, a `VelesQL` value that is not
/// an integer in canonical `VelesQL` form (`WithValue`'s `Display`, which the
/// parser reads back), and the Python `int` itself for one no
/// `i64` holds, which the Python binding refuses before
/// [`parse_with_ef_search`] can read it. Public so that each surface
/// reporting a bad `ef_search` builds this one message instead of a copy.
#[must_use]
pub fn ef_search_out_of_range(shown: impl std::fmt::Display) -> String {
    format!("ef_search must be an integer between {MIN_EF_SEARCH} and {MAX_EF_SEARCH}, got {shown}")
}

/// Validates an `ef_search` value already known to be non-negative (REST, the
/// CLI, the config file) against the documented range. `VelesQL`'s
/// `WITH (ef_search = ...)`, whose grammar accepts a leading `-` that a plain
/// cast would wrap to a huge `usize`, and the Python binding, whose `int` can
/// be negative too, go through [`parse_with_ef_search`] instead (#2274).
///
/// # Errors
///
/// Returns a message naming the accepted range when `ef` falls outside it.
pub fn validate_ef_search(ef: usize) -> Result<(), String> {
    if (MIN_EF_SEARCH..=MAX_EF_SEARCH).contains(&ef) {
        Ok(())
    } else {
        Err(ef_search_out_of_range(ef))
    }
}

/// Parses a signed `ef_search` into a validated `usize`: `VelesQL`'s
/// `WITH (ef_search = ...)`, and the Python binding's `search_with_ef`.
///
/// The `VelesQL` grammar accepts a leading `-` on any integer literal
/// (`grammar.pest`'s `integer` rule), and casting a negative value straight
/// to `usize` wraps it to a huge number — `-1 as usize` is `usize::MAX` — an
/// uncapped graph traversal rather than the refusal this function gives
/// instead (#2274).
///
/// # Errors
///
/// Returns a message naming the accepted range when `ef` is negative or
/// outside it.
pub fn parse_with_ef_search(ef: i64) -> Result<usize, String> {
    usize::try_from(ef)
        .ok()
        .filter(|v| (MIN_EF_SEARCH..=MAX_EF_SEARCH).contains(v))
        .ok_or_else(|| ef_search_out_of_range(ef))
}
