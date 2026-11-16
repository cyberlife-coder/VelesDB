//! The `open` factory and the free-function seams around it: embedder and
//! extractor backend resolution, the ingest-roots allowlist read at open, and
//! the `NodeMemoryService` struct itself. Split out of `lib.rs` when the
//! binding crossed the 1000-line file budget — the `#[napi] impl` surface
//! block stays in `lib.rs` because `binding_parity_bdd` parses it there.

use std::sync::Arc;

use napi_derive::napi;
use velesdb_memory::context::ingest::IngestRoots;
use velesdb_memory::{
    embedder_env_endpoint, role_auth, select_embedder, DynEmbedder, DynExtractor,
    EmbedderSelection, MemoryService, OllamaEmbedder, OllamaExtractor, OpenAiEmbedder,
    OpenAiExtractor, OutlineExtractor, RemoteEndpoint, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL,
    HASH_EMBEDDER_NOTICE,
};

use crate::error::{internal, invalid_input, setting_error};

/// Resolve and build the requested embedder, returning it alongside the model
/// name and semantic flag [`MemoryStore::memory_status`] reports.
///
/// `kind` is the factory's explicit `embedder` argument; when `None`,
/// `VELESDB_MEMORY_EMBEDDER` takes over — the explicit argument always wins,
/// the same precedence and the same [`velesdb_memory::select_embedder`] the
/// daemon resolves the backend name through (#1886: previously this binding
/// read no environment variable at all, so a shell that had semantic recall
/// configured for the MCP daemon silently got the offline default here).
/// `"hash"` is deterministic and offline; `"ollama"` and `"openai"` reach a
/// local or remote embedding model for real semantic recall.
pub(crate) fn build_embedder(
    kind: Option<&str>,
    url: Option<String>,
    model: Option<String>,
) -> napi::Result<(DynEmbedder, String, bool)> {
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
        EmbedderSelection::NeedsRemoteConfig(other) => Err(internal(format!(
            "the embedding backend '{other}' is accepted by velesdb-memory's \
             selector but this binding has no builder for it — this is a bug in \
             velesdb-memory, not a configuration error; please report it quoting this message"
        ))),
    }
}

/// Build the Ollama-backed embedder: an explicit argument wins, then the
/// environment (honouring the legacy `VELESDB_MEMORY_OLLAMA_*` aliases, C1),
/// then the built-in local defaults.
pub(crate) fn build_ollama_embedder(
    url: Option<String>,
    model: Option<String>,
) -> napi::Result<(DynEmbedder, String)> {
    let (env_endpoint, _alias_conflict) = embedder_env_endpoint().map_err(setting_error)?;
    let url = url
        .or(env_endpoint.url)
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = model
        .or(env_endpoint.model)
        .unwrap_or_else(|| DEFAULT_OLLAMA_MODEL.to_owned());
    // A malformed URL is caller configuration (argument or environment), so
    // it reports INVALID_INPUT like every other setting error — not INTERNAL.
    let embedder =
        OllamaEmbedder::new(url, model.clone()).map_err(|e| invalid_input(e.to_string()))?;
    Ok((Box::new(embedder), model))
}

/// Build the OpenAI-compatible embedder. The URL, model and credential come
/// from the environment only — never from a factory argument, so a caller can
/// never end up with a token sitting in their own source file — see
/// [`velesdb_memory::RemoteEndpoint::require`].
pub(crate) fn build_openai_embedder() -> napi::Result<(DynEmbedder, String)> {
    let (env_endpoint, _alias_conflict) = embedder_env_endpoint().map_err(setting_error)?;
    let (url, model, auth) = env_endpoint
        .require("VELESDB_MEMORY_EMBEDDER")
        .map_err(setting_error)?;
    // Same as the Ollama builder: a URL the transport refuses is caller
    // configuration, reported INVALID_INPUT like the `require` above it.
    let embedder =
        OpenAiEmbedder::new(url, model.clone(), auth).map_err(|e| invalid_input(e.to_string()))?;
    Ok((Box::new(embedder), model))
}

/// The extraction role's environment endpoint
/// (`VELESDB_MEMORY_EXTRACTOR_URL` / `_MODEL` / `_API_TOKEN`), built from the
/// same public pieces the daemon uses — the struct fields are public and
/// [`role_auth`] resolves the credential — so the binding resolves the role
/// exactly like the MCP server does.
pub(crate) fn extractor_env_endpoint() -> napi::Result<RemoteEndpoint> {
    Ok(RemoteEndpoint {
        url: std::env::var("VELESDB_MEMORY_EXTRACTOR_URL").ok(),
        model: std::env::var("VELESDB_MEMORY_EXTRACTOR_MODEL").ok(),
        auth: role_auth("VELESDB_MEMORY_EXTRACTOR_API_TOKEN").map_err(setting_error)?,
    })
}

/// Read `VELESDB_MEMORY_INGEST_ROOTS` once at `open`: the canonical allowlist
/// under which `path` fragments and file anchors may be read. Unset/empty
/// means disabled — every `path` then fails closed with
/// [`MemoryError::IngestDisabled`], and anchoring with [`AnchorError`].
/// A malformed value is an operator error, rejected at open like any other
/// bad setting.
///
/// [`AnchorError`]: velesdb_memory::anchor::AnchorError
pub(crate) fn ingest_roots_from_env() -> napi::Result<Option<IngestRoots>> {
    match std::env::var("VELESDB_MEMORY_INGEST_ROOTS") {
        Ok(value) if !value.trim().is_empty() => IngestRoots::parse(&value)
            .map(Some)
            .map_err(|e| invalid_input(e.to_string())),
        _ => Ok(None),
    }
}

/// Surface the lexical fallback once per successfully opened Node instance.
pub(crate) fn warn_hash_embedder_not_semantic(semantic: bool) {
    if semantic || std::env::var_os("VELESDB_MEMORY_QUIET").is_some() {
        return;
    }
    eprintln!(
        "[velesdb-memory-node] {HASH_EMBEDDER_NOTICE} For real semantic recall \
         reopen with MemoryService.open(path, \"ollama\") or configure \
         VELESDB_MEMORY_EMBEDDER=ollama. Set VELESDB_MEMORY_QUIET=1 to silence this notice."
    );
}

/// Local-first agent memory with the `why()` graph wedge.
///
/// All methods are async (return a Promise) and run off the event-loop thread.
///
/// Exposed to JS as `MemoryService` (matching the `PyO3` binding and the core
/// type); the Rust struct keeps a distinct name only to avoid colliding with the
/// imported [`velesdb_memory::MemoryService`] it wraps. `NodeMemoryService`
/// rather than the old `MemoryStore`: that name collided with
/// `velesdb_memory`'s storage-trait alias on the other side of the binding
/// seam, exactly where ambiguity costs the most (#2018) — and being
/// `js_name`-mapped, the Rust identifier is invisible to JS consumers.
#[napi(js_name = "MemoryService")]
pub struct NodeMemoryService {
    pub(crate) inner: Arc<MemoryService<DynEmbedder>>,
    /// The embedder identity resolved at [`Self::open`] — what
    /// [`Self::memory_status`] reports as RUNNING. The service itself only
    /// ever sees `&[f32]`, so the factory is the one place that knows.
    pub(crate) embedder_model: String,
    pub(crate) embedder_dimension: usize,
    pub(crate) embedder_semantic: bool,
    /// Where the store lives, for the provenance block of
    /// [`Self::memory_status`] (#1751's on-disk record).
    pub(crate) store_dir: std::path::PathBuf,
    /// The canonical ingest-roots allowlist read from
    /// `VELESDB_MEMORY_INGEST_ROOTS` at [`Self::open`] — the same variable the
    /// daemon reads at startup. Gates `path` fragments in the compile methods
    /// and the file anchors of `remember`/`recall`; `None` disables both,
    /// fail-closed. The containment check itself always runs at the sink
    /// (`AnchorRoot::open` / `ingest::resolve_fragments`), never here.
    pub(crate) ingest_roots: Option<IngestRoots>,
}

/// Build the extraction backend named `backend` for one `rememberExtracted`
/// call. Explicit `model`/`url` arguments win over the extraction role's
/// environment (`VELESDB_MEMORY_EXTRACTOR_*`); the OpenAI credential comes
/// from `VELESDB_MEMORY_EXTRACTOR_API_TOKEN` only — never from an argument,
/// so no token lands in a source file.
pub(crate) fn build_extractor(
    backend: &str,
    model: Option<String>,
    url: Option<String>,
) -> napi::Result<DynExtractor> {
    match backend {
        "outline" => Ok(Arc::new(OutlineExtractor)),
        "ollama" => build_ollama_extractor(model, url),
        "openai" => build_openai_extractor(model, url),
        "none" | "" => Err(invalid_input(
            "extractor 'none' cannot extract a passage; pass 'outline' for the \
             offline backend, or 'ollama'/'openai' with configuration",
        )),
        other => Err(invalid_input(format!(
            "unknown extractor '{other}' (expected 'ollama', 'outline' or 'openai')"
        ))),
    }
}

/// The `ollama` extractor: explicit `url`/`model` win over
/// `VELESDB_MEMORY_EXTRACTOR_*`, then the default Ollama URL.
fn build_ollama_extractor(
    model: Option<String>,
    url: Option<String>,
) -> napi::Result<DynExtractor> {
    let env_endpoint = extractor_env_endpoint()?;
    let url = url
        .or(env_endpoint.url)
        .unwrap_or_else(|| DEFAULT_OLLAMA_URL.to_owned());
    let model = model.or(env_endpoint.model).ok_or_else(|| {
        invalid_input(
            "the 'ollama' extractor needs a `model` (argument or \
             VELESDB_MEMORY_EXTRACTOR_MODEL; or pass extractor: 'outline' \
             for the offline backend)",
        )
    })?;
    Ok(Arc::new(OllamaExtractor::new(url, model)))
}

/// The `openai` extractor: URL/model from the argument or
/// `VELESDB_MEMORY_EXTRACTOR_*`, credential from the environment only.
fn build_openai_extractor(
    model: Option<String>,
    url: Option<String>,
) -> napi::Result<DynExtractor> {
    let env_endpoint = extractor_env_endpoint()?;
    let (url, model, auth) = RemoteEndpoint {
        url: url.or(env_endpoint.url),
        model: model.or(env_endpoint.model),
        auth: env_endpoint.auth,
    }
    .require("VELESDB_MEMORY_EXTRACTOR")
    .map_err(setting_error)?;
    Ok(Arc::new(OpenAiExtractor::new(url, model, auth)))
}

/// The allowlist handed to [`velesdb_memory::anchor::AnchorRoot::open`] /
/// [`velesdb_memory::anchor::AnchorRoot::resolve_request`], empty when ingestion is disabled.
/// (A free fn, not an inherent method: `binding_parity_bdd` parses the ONE
/// `impl NodeMemoryService {` surface block, and a second inherent impl would
/// truncate that scan.)
pub(crate) fn anchor_allowlist(ingest_roots: Option<&IngestRoots>) -> Vec<std::path::PathBuf> {
    ingest_roots.map_or_else(Vec::new, |roots| roots.paths().to_vec())
}
