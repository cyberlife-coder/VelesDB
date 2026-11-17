//! Choosing the embedder and the extractor the service is built with.

use super::errors::{setting_error, to_py_err};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use velesdb_memory::{
    embedder_env_endpoint, extractor_env_endpoint, limits, select_embedder, select_extractor,
    DynEmbedder, DynExtractor, EmbedderSelection, ExtractorSelection, MemoryService, Metadata,
    OllamaEmbedder, OllamaExtractor, OpenAiEmbedder, OpenAiExtractor, RememberedExtraction,
    RemoteEndpoint, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL, HASH_EMBEDDER_NOTICE,
};

/// Resolve and build the requested embedder, returning it alongside the model
/// name and semantic flag `memory_status` reports.
///
/// `kind` is the constructor's explicit `embedder` argument; when `None`,
/// `VELESDB_MEMORY_EMBEDDER` takes over — the explicit argument always wins,
/// the same precedence and the same [`velesdb_memory::select_embedder`] the
/// daemon resolves the backend name through (#1886: previously this binding
/// read no environment variable at all, so a shell that had semantic recall
/// configured for the MCP daemon silently got the offline default here).
/// `"hash"` is deterministic and offline; `"ollama"` and `"openai"` reach a
/// local or remote embedding model for real semantic recall.
pub(super) fn build_embedder(
    kind: Option<&str>,
    url: Option<String>,
    model: Option<String>,
) -> PyResult<(DynEmbedder, String, bool)> {
    let backend_env = std::env::var("VELESDB_MEMORY_EMBEDDER").ok();
    let selection = select_embedder(kind.or(backend_env.as_deref())).map_err(setting_error)?;
    match selection {
        EmbedderSelection::Ready(name, embedder) => Ok((embedder, name.to_owned(), name != "hash")),
        EmbedderSelection::NeedsRemoteConfig("ollama") => {
            build_ollama_embedder(url, model).map(|(embedder, model)| (embedder, model, true))
        }
        EmbedderSelection::NeedsRemoteConfig("openai") => {
            build_openai_embedder().map(|(embedder, model)| (embedder, model, true))
        }
        EmbedderSelection::NeedsRemoteConfig(other) => Err(PyRuntimeError::new_err(format!(
            "the embedding backend '{other}' is accepted by velesdb-memory's selector but \
             this binding has no builder for it — this is a bug in velesdb-memory, not a \
             configuration error; please report it quoting this message"
        ))),
    }
}

/// Build the Ollama-backed embedder: an explicit argument wins, then the
/// environment (honouring the legacy `VELESDB_MEMORY_OLLAMA_*` aliases, C1),
/// then the built-in local defaults.
fn build_ollama_embedder(
    url: Option<String>,
    model: Option<String>,
) -> PyResult<(DynEmbedder, String)> {
    let (env_endpoint, _alias_conflict) = embedder_env_endpoint().map_err(setting_error)?;
    let url = url
        .or(env_endpoint.url)
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = model
        .or(env_endpoint.model)
        .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
    // A malformed URL is the caller's setting: ValueError, as Node answers
    // INVALID_INPUT.
    let embedder = OllamaEmbedder::new(url, model.clone())
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok((Box::new(embedder), model))
}

/// Build the OpenAI-compatible embedder. The URL, model and credential come
/// from the environment only — never from a constructor argument, so a
/// caller can never end up with a token sitting in their own source file —
/// see [`velesdb_memory::RemoteEndpoint::require`].
fn build_openai_embedder() -> PyResult<(DynEmbedder, String)> {
    let (env_endpoint, _alias_conflict) = embedder_env_endpoint().map_err(setting_error)?;
    let (url, model, auth) = env_endpoint
        .require("VELESDB_MEMORY_EMBEDDER")
        .map_err(setting_error)?;
    let embedder = OpenAiEmbedder::new(url, model.clone(), auth)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok((Box::new(embedder), model))
}

/// Surface the lexical fallback once per successfully opened Python instance.
pub(super) fn warn_hash_embedder_not_semantic(semantic: bool) {
    if semantic || std::env::var_os("VELESDB_MEMORY_QUIET").is_some() {
        return;
    }
    eprintln!(
        "[velesdb] {HASH_EMBEDDER_NOTICE} For real semantic recall reopen with \
         MemoryService(path, embedder=\"ollama\") or configure \
         VELESDB_MEMORY_EMBEDDER=ollama. Set VELESDB_MEMORY_QUIET=1 to silence this notice."
    );
}

/// Build the requested extractor and run it, releasing the GIL for the call.
///
/// The same contract as the Node binding and the MCP tool: a passage over
/// [`limits::MAX_FACT_BYTES`] is refused before any backend sees it; the
/// backend is the explicit `extractor`, else `VELESDB_MEMORY_EXTRACTOR`, else
/// `"ollama"`, resolved by `velesdb_memory`'s [`select_extractor`] (so an
/// unknown name is a `ValueError` naming the accepted forms); the remote
/// backends take `url`/`model` from the arguments, else
/// `VELESDB_MEMORY_EXTRACTOR_URL` / `_MODEL`, and a credential from the
/// environment only. A setting the caller got wrong is a `ValueError`.
///
/// A free function rather than a method, so it stays out of the `impl` block
/// the binding-parity guard reads as this binding's published surface.
pub(super) fn extract_with(
    py: Python<'_>,
    svc: &MemoryService<DynEmbedder>,
    text: &str,
    model: Option<String>,
    url: Option<String>,
    extractor: Option<String>,
    metadata: Option<Metadata>,
) -> PyResult<RememberedExtraction> {
    if text.len() > limits::MAX_FACT_BYTES {
        return Err(PyValueError::new_err(format!(
            "text exceeds {} bytes ({} given)",
            limits::MAX_FACT_BYTES,
            text.len()
        )));
    }
    let backend = extractor
        .or_else(|| std::env::var("VELESDB_MEMORY_EXTRACTOR").ok())
        .unwrap_or_else(|| "ollama".to_owned());
    let extractor: DynExtractor = match select_extractor(&backend).map_err(setting_error)? {
        ExtractorSelection::Ready(extractor) => extractor,
        ExtractorSelection::Disabled => {
            return Err(PyValueError::new_err(
                "extractor 'none' cannot extract a passage; pass 'outline' for the offline \
                 backend, or 'ollama'/'openai' with configuration",
            ))
        }
        ExtractorSelection::NeedsRemoteConfig(name) => build_remote_extractor(name, model, url)?,
    };
    py.detach(|| {
        svc.remember_extracted(text, &extractor, metadata.as_ref())
            .map_err(to_py_err)
    })
}

/// A network-backed extractor: `url`/`model` from the arguments, else the
/// extraction role's environment endpoint.
fn build_remote_extractor(
    name: &str,
    model: Option<String>,
    url: Option<String>,
) -> PyResult<DynExtractor> {
    let env_endpoint = extractor_env_endpoint().map_err(setting_error)?;
    match name {
        "ollama" => {
            let model = model.or(env_endpoint.model).ok_or_else(|| {
                PyValueError::new_err(
                    "the 'ollama' extractor needs a model (argument or \
                     VELESDB_MEMORY_EXTRACTOR_MODEL; or pass extractor='outline' for the \
                     offline backend)",
                )
            })?;
            let url = url
                .or(env_endpoint.url)
                .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
            Ok(std::sync::Arc::new(OllamaExtractor::new(url, model)))
        }
        "openai" => {
            let (url, model, auth) = RemoteEndpoint {
                url: url.or(env_endpoint.url),
                model: model.or(env_endpoint.model),
                auth: env_endpoint.auth,
            }
            .require("VELESDB_MEMORY_EXTRACTOR")
            .map_err(setting_error)?;
            Ok(std::sync::Arc::new(OpenAiExtractor::new(url, model, auth)))
        }
        other => Err(PyRuntimeError::new_err(format!(
            "extractor '{other}' needs remote configuration this binding has no builder \
             for — a bug in velesdb-memory, not a configuration error; please report it"
        ))),
    }
}
