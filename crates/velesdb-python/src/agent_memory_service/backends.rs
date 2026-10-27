//! Choosing the embedder and the extractor the service is built with.

use super::errors::to_py_err;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use velesdb_memory::{
    embedder_env_endpoint, select_embedder, DynEmbedder, EmbedderSelection, MemoryService,
    Metadata, OllamaEmbedder, OllamaExtractor, OpenAiEmbedder, OutlineExtractor,
    RememberedExtraction, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL, HASH_EMBEDDER_NOTICE,
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
    let selection =
        select_embedder(kind.or(backend_env.as_deref())).map_err(PyValueError::new_err)?;
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
    let (env_endpoint, _alias_conflict) = embedder_env_endpoint().map_err(PyValueError::new_err)?;
    let url = url
        .or(env_endpoint.url)
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = model
        .or(env_endpoint.model)
        .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
    let embedder = OllamaEmbedder::new(url, model.clone())
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
    Ok((Box::new(embedder), model))
}

/// Build the OpenAI-compatible embedder. The URL, model and credential come
/// from the environment only — never from a constructor argument, so a
/// caller can never end up with a token sitting in their own source file —
/// see [`velesdb_memory::RemoteEndpoint::require`].
fn build_openai_embedder() -> PyResult<(DynEmbedder, String)> {
    let (env_endpoint, _alias_conflict) = embedder_env_endpoint().map_err(PyValueError::new_err)?;
    let (url, model, auth) = env_endpoint
        .require("VELESDB_MEMORY_EMBEDDER")
        .map_err(PyValueError::new_err)?;
    let embedder = OpenAiEmbedder::new(url, model.clone(), auth)
        .map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
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
/// Same contract as [`build_embedder`], including the `ValueError` on an
/// unknown name: a caller who asked for a backend that does not exist must be
/// told so, never handed a different one. `"outline"` is deterministic and
/// network-free and ignores `model`/`url`; `"ollama"` needs a `model`.
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
    match extractor.as_deref().unwrap_or("ollama") {
        "outline" => py.detach(|| {
            svc.remember_extracted(text, &OutlineExtractor, metadata.as_ref())
                .map_err(to_py_err)
        }),
        "ollama" => {
            let model = model.ok_or_else(|| {
                PyValueError::new_err(
                    "the 'ollama' extractor needs a model (or pass extractor='outline' for \
                     the offline backend)",
                )
            })?;
            let url = url.unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
            let backend = OllamaExtractor::new(url, model);
            py.detach(|| {
                svc.remember_extracted(text, &backend, metadata.as_ref())
                    .map_err(to_py_err)
            })
        }
        other => Err(PyValueError::new_err(format!(
            "unknown extractor '{other}' (expected 'ollama' or 'outline')"
        ))),
    }
}
