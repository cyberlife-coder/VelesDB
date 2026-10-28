//! Which recall quality the operator asked for.
//!
//! Recall searches the vector index at the collection's runtime quality unless
//! a quality is configured here. The grammar is the one REST, VelesQL, the CLI
//! and the Python SDK already speak, delegated to
//! [`velesdb_core::api_types::parse_search_mode`] so a mode means the same
//! thing, and fails with the same words, on every surface.

use crate::setting::SettingError;
use crate::SearchQuality;

/// The variable the daemon reads (`[search] quality` in the config file).
pub const SEARCH_QUALITY_VAR: &str = "VELESDB_MEMORY_SEARCH_QUALITY";

/// Parse a configured quality.
///
/// `None`, empty and blank all mean "not configured" and return `Ok(None)`:
/// the collection keeps its own runtime quality, which is what every store did
/// before this setting existed. Anything else must be a quality, or startup
/// stops: a setting dropped with a warning is how an operator keeps believing
/// recall runs at a quality it does not.
///
/// # Errors
/// [`SettingError::InvalidSearchQuality`] naming the accepted forms.
pub fn select_search_quality(mode: Option<&str>) -> Result<Option<SearchQuality>, SettingError> {
    let Some(mode) = mode.map(str::trim).filter(|mode| !mode.is_empty()) else {
        return Ok(None);
    };
    velesdb_core::api_types::parse_search_mode(mode)
        .map(Some)
        .map_err(|detail| SettingError::InvalidSearchQuality { detail })
}

#[cfg(test)]
#[path = "search_quality_tests.rs"]
mod tests;
