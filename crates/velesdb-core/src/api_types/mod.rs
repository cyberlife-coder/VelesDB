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
/// Returns `None` for a mode it cannot read, or whose ef falls outside the
/// `ef_search` range. An entry point that must refuse such a mode rather than
/// fall back uses [`parse_search_mode`], which says why.
#[cfg(feature = "persistence")]
#[must_use]
pub fn mode_to_search_quality(mode: &str) -> Option<crate::SearchQuality> {
    parse_search_mode(mode).ok()
}

/// The accepted search mode forms, as every mode error names them.
#[cfg(feature = "persistence")]
pub(crate) const SEARCH_MODE_FORMS: &str = concat!(
    "Valid values: 'fast', 'balanced', 'accurate', 'perfect', ",
    "'autotune' (aliases: 'auto_tune', 'auto'), 'custom:<ef>', ",
    "'adaptive:<min_ef>:<max_ef>' (min_ef <= max_ef), each ef in the ef_search range"
);

/// Parses a search mode string into a [`crate::SearchQuality`], or an error
/// saying why it cannot be used.
///
/// Unlike [`mode_to_search_quality`], an unusable mode is a distinct `Err`
/// here rather than a `None` a caller might mistake for "no mode given". Use
/// this at entry points where the caller should reject a typo instead of
/// silently falling back to the default quality (#2267).
///
/// # Errors
///
/// Returns a message naming the accepted forms when `mode` matches none of
/// them (an unknown name, or `custom:`/`adaptive:` with a malformed or
/// out-of-order argument), and one naming the value when a `custom:` or
/// `adaptive:` ef is an integer outside `[MIN_EF_SEARCH, MAX_EF_SEARCH]`,
/// the range the dedicated `ef_search` option enforces (#2275). The range
/// is checked first: `adaptive:5000:32` names 5000.
#[cfg(feature = "persistence")]
pub fn parse_search_mode(mode: &str) -> Result<crate::SearchQuality, String> {
    let quality = match mode.to_lowercase().as_str() {
        "fast" => Some(crate::SearchQuality::Fast),
        "balanced" => Some(crate::SearchQuality::Balanced),
        "accurate" => Some(crate::SearchQuality::Accurate),
        "perfect" => Some(crate::SearchQuality::Perfect),
        "autotune" | "auto_tune" | "auto" => Some(crate::SearchQuality::AutoTune),
        other => parse_advanced_quality(other)
            .map_err(|out_of_range| format!("Search mode '{mode}': {out_of_range}"))?,
    };
    quality.ok_or_else(|| format!("Unknown search mode '{mode}'. {SEARCH_MODE_FORMS}"))
}

/// Parses the advanced search quality modes, `custom:<ef>` and
/// `adaptive:<min_ef>:<max_ef>`: `Err` when an ef is an integer out of
/// range, which is checked before the order of the adaptive bounds, since no
/// order makes such a bound valid; `Ok(None)` for a malformed mode, or an
/// out-of-order one whose bounds are both in range.
#[cfg(feature = "persistence")]
fn parse_advanced_quality(mode: &str) -> Result<Option<crate::SearchQuality>, String> {
    if let Some(ef) = mode.strip_prefix("custom:") {
        return Ok(parse_mode_ef(ef)
            .transpose()?
            .map(crate::SearchQuality::Custom));
    }
    let Some((min_ef, max_ef)) = mode
        .strip_prefix("adaptive:")
        .and_then(|bounds| bounds.split_once(':'))
    else {
        return Ok(None);
    };
    // Both must be integers before either range is checked: a malformed mode
    // is an unknown one, whatever its other bound holds.
    let (Some(min_ef), Some(max_ef)) = (parse_mode_ef(min_ef), parse_mode_ef(max_ef)) else {
        return Ok(None);
    };
    let (min_ef, max_ef) = (min_ef?, max_ef?);
    Ok((min_ef <= max_ef).then_some(crate::SearchQuality::Adaptive { min_ef, max_ef }))
}

/// Reads one ef of an advanced search mode: `None` when `raw` is not an
/// integer, `Some(Err)` when it is one outside the `ef_search` range: a
/// negative one, and one past `usize::MAX`, included, as `WITH (ef_search =
/// ...)` reads them.
#[cfg(feature = "persistence")]
fn parse_mode_ef(raw: &str) -> Option<Result<usize, String>> {
    use std::num::IntErrorKind::{NegOverflow, PosOverflow};
    let in_range = |ef: usize| validate_ef_search(ef).map(|()| ef);
    match raw.parse::<i128>() {
        Ok(ef) => {
            Some(usize::try_from(ef).map_or_else(|_| Err(ef_search_out_of_range(raw)), in_range))
        }
        Err(e) if matches!(e.kind(), PosOverflow | NegOverflow) => {
            Some(Err(ef_search_out_of_range(raw)))
        }
        Err(_) => None,
    }
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
