//! Resolving an extraction backend name to what the caller must do about it.

use super::{DynExtractor, OutlineExtractor};
use crate::setting::SettingError;

/// What a caller must do to honour a requested extraction backend.
///
/// Returned by [`select_extractor`], which is the single place that knows which
/// backend names exist. Splitting the answer into these three shapes is what
/// lets the dependency-free backends be selected in **any** build: only the
/// [`Self::NeedsRemoteConfig`] arm requires an optional dependency and the URL
/// and model that go with it, and only that arm's construction is feature-gated.
/// **Deliberately exhaustive** (no `non_exhaustive`): every variant demands
/// caller wiring — construct a backend, ask for configuration, run nothing —
/// and a wildcard arm would silently ignore a new capability instead of
/// failing to compile where it must be handled. Adding a variant is therefore
/// a breaking change, made on purpose, in a minor bump while the crate is 0.x.
pub enum ExtractorSelection {
    /// No extraction. Tools that need an extractor answer "not configured".
    Disabled,
    /// Ready to use as-is: needs no configuration, no network, no optional
    /// dependency. Attach it and the graph builds.
    Ready(DynExtractor),
    /// A network-backed backend the caller must build itself, because only the
    /// caller knows its URL and model. Carries the backend's name so the caller
    /// can dispatch without re-parsing the string.
    NeedsRemoteConfig(&'static str),
}

/// Hand-written because [`DynExtractor`] is a trait object and the trait does
/// not require `Debug` — a backend is identified by its shape here, never by
/// dumping its innards (an HTTP-backed one holds a URL, and a panic message is
/// not the place for it).
impl std::fmt::Debug for ExtractorSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("Disabled"),
            Self::Ready(_) => f.write_str("Ready(<extractor>)"),
            Self::NeedsRemoteConfig(name) => write!(f, "NeedsRemoteConfig({name})"),
        }
    }
}

/// Resolve an extraction backend name to what the caller must do about it.
///
/// # Why this exists, and why it is in the library rather than the binary
///
/// The selection used to live inside the daemon's `#[cfg(feature = "extractor-http")]`
/// block. That gate is what made [`OutlineExtractor`] unreachable from the MCP
/// server (#1734): the extractor needs no dependency and is linked into every
/// build, but the only code that could *choose* it was compiled away unless an
/// unrelated HTTP feature was on. Two of the twenty published tools were dead by
/// default as a result — `remember_extracted` refused outright, and `entity`
/// answered `found: false` for every name, entity hubs being born only of
/// extraction.
///
/// Living here rather than in `main.rs` also means the daemon and the tests
/// exercise the **same** function: a test can select `outline` and drive the
/// real server with the result, instead of proving a seam written for the test.
///
/// # Errors
/// [`SettingError::UnknownExtractor`], whose message names the accepted forms, for an
/// unknown backend.
pub fn select_extractor(backend: &str) -> Result<ExtractorSelection, SettingError> {
    match backend {
        // No `#[cfg]` here, and that absence IS the fix: this arm must survive
        // in a build without any HTTP feature, which is exactly the build the
        // published binary ships.
        "outline" => Ok(ExtractorSelection::Ready(std::sync::Arc::new(
            OutlineExtractor,
        ))),
        "ollama" => Ok(ExtractorSelection::NeedsRemoteConfig("ollama")),
        // A protocol, not a vendor — see [`crate::select_embedder`]'s own
        // `openai` arm. The two roles accept the same names on purpose: an
        // operator who learned one has learned the other.
        "openai" => Ok(ExtractorSelection::NeedsRemoteConfig("openai")),
        "none" | "" => Ok(ExtractorSelection::Disabled),
        other => Err(SettingError::UnknownExtractor {
            name: other.to_owned(),
        }),
    }
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
