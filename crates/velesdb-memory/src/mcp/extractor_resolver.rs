//! Resolve the default and per-call extraction backends for MCP tools.

use crate::extract::{select_extractor, DynExtractor, ExtractorSelection};

/// Failure to resolve the extractor for one `remember_extracted` call.
pub(super) enum ExtractorResolveError {
    /// No per-call choice and no daemon-level default.
    DefaultNotConfigured,
    /// The caller requested a name this running server cannot serve.
    InvalidRequest(String),
}

#[derive(Clone)]
struct NamedExtractor {
    backend: Option<String>,
    extractor: DynExtractor,
}

/// The daemon-level default plus the rules for a per-call override.
#[derive(Clone, Default)]
pub(super) struct ExtractorResolver {
    default: Option<NamedExtractor>,
}

impl ExtractorResolver {
    /// Attach an unnamed default for compatibility with embedded servers.
    pub(super) fn unnamed(extractor: DynExtractor) -> Self {
        Self {
            default: Some(NamedExtractor {
                backend: None,
                extractor,
            }),
        }
    }

    /// Attach and name the backend selected by the daemon at startup.
    pub(super) fn named(backend: String, extractor: DynExtractor) -> Result<Self, String> {
        match select_extractor(&backend)? {
            ExtractorSelection::Disabled => {
                Err("a disabled extractor cannot be attached as the default".to_owned())
            }
            ExtractorSelection::Ready(_) | ExtractorSelection::NeedsRemoteConfig(_) => Ok(Self {
                default: Some(NamedExtractor {
                    backend: Some(backend),
                    extractor,
                }),
            }),
        }
    }

    /// Resolve an optional per-call name, falling back to the daemon default.
    pub(super) fn resolve(
        &self,
        requested: Option<&str>,
    ) -> Result<DynExtractor, ExtractorResolveError> {
        match requested {
            Some(backend) => self.resolve_requested(backend),
            None => self
                .default
                .as_ref()
                .map(|configured| configured.extractor.clone())
                .ok_or(ExtractorResolveError::DefaultNotConfigured),
        }
    }

    /// Whether omitting the per-call choice has a configured default.
    pub(super) fn default_is_configured(&self) -> bool {
        self.default.is_some()
    }

    fn resolve_requested(&self, backend: &str) -> Result<DynExtractor, ExtractorResolveError> {
        match select_extractor(backend).map_err(ExtractorResolveError::InvalidRequest)? {
            ExtractorSelection::Ready(extractor) => Ok(extractor),
            ExtractorSelection::NeedsRemoteConfig(name) => self.resolve_remote(name),
            ExtractorSelection::Disabled => Err(ExtractorResolveError::InvalidRequest(
                "extractor 'none' cannot extract a passage; omit the field to use the daemon default"
                    .to_owned(),
            )),
        }
    }

    fn resolve_remote(&self, backend: &str) -> Result<DynExtractor, ExtractorResolveError> {
        let Some(configured) = &self.default else {
            return Err(ExtractorResolveError::InvalidRequest(format!(
                "extractor '{backend}' is not configured for this server; start it with \
                 VELESDB_MEMORY_EXTRACTOR={backend} or request 'outline'"
            )));
        };
        if configured.backend.as_deref() == Some(backend) {
            return Ok(configured.extractor.clone());
        }
        let current = configured
            .backend
            .as_deref()
            .unwrap_or("an unnamed backend");
        Err(ExtractorResolveError::InvalidRequest(format!(
            "extractor '{backend}' is unavailable; this server uses {current} as its default — \
             restart it with VELESDB_MEMORY_EXTRACTOR={backend} or request 'outline'"
        )))
    }
}
