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

/// The quality to run at: an explicit one (a binding's argument) wins over
/// [`SEARCH_QUALITY_VAR`], which wins over "not configured".
///
/// Mirrors how the bindings already resolve the embedder: the caller who named
/// a value gets exactly that value, and only a caller who named nothing is
/// handed the operator's environment.
///
/// # Errors
/// [`SettingError::InvalidSearchQuality`] when the value in force is not a
/// quality, or the variable is not valid unicode.
pub fn resolve_search_quality(
    explicit: Option<&str>,
) -> Result<Option<SearchQuality>, SettingError> {
    let environment = match std::env::var(SEARCH_QUALITY_VAR) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(SettingError::InvalidSearchQuality {
                detail: format!("{SEARCH_QUALITY_VAR} is not valid unicode"),
            })
        }
    };
    resolve(explicit, environment.as_deref())
}

/// [`resolve_search_quality`] with the environment's value passed in.
fn resolve(
    explicit: Option<&str>,
    environment: Option<&str>,
) -> Result<Option<SearchQuality>, SettingError> {
    match explicit.map(str::trim).filter(|mode| !mode.is_empty()) {
        Some(mode) => select_search_quality(Some(mode)),
        None => select_search_quality(environment),
    }
}

#[cfg(test)]
#[path = "search_quality_tests.rs"]
mod tests;
