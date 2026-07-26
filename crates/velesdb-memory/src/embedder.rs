//! Pluggable text → vector embedding.
//!
//! The Agent Memory SDK is *bring-your-own-vector*: it never generates
//! embeddings. This crate mirrors the repo's established pattern (the Python
//! SDK's `Embedder` protocol, the tauri-rag demo's `fastembed` backend): an
//! [`Embedder`] trait with a default on-device model and a deterministic,
//! network-free fallback for tests and air-gapped reproducibility.

#[cfg(feature = "ollama")]
use serde::Deserialize;

/// Failure produced by an [`Embedder`] backend (e.g. a network-backed embedder
/// that cannot reach its model). The in-memory [`HashEmbedder`] never fails.
#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    /// The embedding backend (network, subprocess, …) returned an error.
    #[error("embedding backend error: {0}")]
    Backend(String),
    /// The backend returned an empty embedding vector.
    #[error("embedding backend returned an empty vector")]
    Empty,
}

/// Turns text into a fixed-dimension embedding vector.
pub trait Embedder {
    /// Embedding dimension produced by [`Embedder::embed`].
    fn dimension(&self) -> usize;

    /// Embed `text` into a vector of length [`Embedder::dimension`].
    ///
    /// # Errors
    /// Returns [`EmbedError`] if the backend cannot produce an embedding.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>;
}

/// Deterministic, network-free embedder (token-hashing into L2-normalized
/// buckets). Not semantically strong — its purpose is reproducible tests and
/// offline behavior, exactly like the `fake_embed` used in the repo's
/// `agent_memory` examples. Swap in a real model (e.g. `fastembed`,
/// all-MiniLM-L6-v2, 384-dim) for production recall quality.
#[derive(Debug, Clone)]
pub struct HashEmbedder {
    dimension: usize,
}

impl HashEmbedder {
    /// Create a [`HashEmbedder`] producing vectors of `dimension` length.
    /// Use `384` to match the SDK's `DEFAULT_DIMENSION`.
    #[must_use]
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

impl Embedder for HashEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut vector = vec![0.0_f32; self.dimension];
        if self.dimension == 0 {
            return Ok(vector);
        }
        let modulus = self.dimension as u64;
        for token in text.split_whitespace() {
            let bucket = usize::try_from(crate::id::stable_id(token) % modulus).unwrap_or(0);
            vector[bucket] += 1.0;
        }
        velesdb_core::simd_native::normalize_inplace_native(&mut vector);
        Ok(vector)
    }
}

/// A boxed, object-safe embedder. Lets a non-generic `MemoryService<DynEmbedder>`
/// be stored behind a concrete type — the MCP server and the language bindings
/// both need this, since handler/pyclass types can't carry a generic `E`.
pub type DynEmbedder = Box<dyn Embedder + Send + Sync>;

/// Forward [`Embedder`] through a box, enabling a non-generic
/// `MemoryService<DynEmbedder>` for the MCP server and bindings.
impl<T: Embedder + ?Sized> Embedder for Box<T> {
    fn dimension(&self) -> usize {
        (**self).dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        (**self).embed(text)
    }
}

// --- Optional real-recall backend: a local Ollama embeddings endpoint --------
//
// Enabled with `--features ollama`. The default build omits this backend (and
// its HTTP dependency) so the shipped binary stays tiny, zero-dependency, and
// fully offline. This backend keeps the binary small too: it calls a model the
// user already runs locally, so the memory still never leaves the machine.

/// Default Ollama base URL.
#[cfg(feature = "ollama")]
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Default Ollama embedding model (384-dim; `ollama pull all-minilm`).
#[cfg(feature = "ollama")]
pub const DEFAULT_OLLAMA_MODEL: &str = "all-minilm";

/// Embeds text through a local Ollama `/api/embeddings` endpoint — real
/// semantic recall while the model stays on the user's own machine.
#[cfg(feature = "ollama")]
#[derive(Debug, Clone)]
pub struct OllamaEmbedder {
    base_url: String,
    model: String,
    dimension: usize,
    agent: ureq::Agent,
}

#[cfg(feature = "ollama")]
impl OllamaEmbedder {
    /// Connect to Ollama at `base_url` using `model`, probing the embedding
    /// dimension once so it adapts to whatever model is configured.
    ///
    /// # Errors
    /// Returns [`EmbedError`] if Ollama is unreachable or the model does not
    /// produce embeddings.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Result<Self, EmbedError> {
        let base_url = base_url.into();
        let model = model.into();
        let agent = embed_agent(std::time::Duration::from_secs(EMBED_TIMEOUT_SECS));
        let dimension = request_embedding(&agent, &base_url, &model, "dimension probe")?.len();
        if dimension == 0 {
            return Err(EmbedError::Empty);
        }
        Ok(Self {
            base_url,
            model,
            dimension,
            agent,
        })
    }
}

#[cfg(feature = "ollama")]
impl Embedder for OllamaEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        request_embedding(&self.agent, &self.base_url, &self.model, text)
    }
}

/// Build the JSON request body for the embeddings endpoint.
/// How long Ollama keeps a model resident after a request. `-1` means "for as
/// long as the server runs", which is what a daemon wants: the model loads once
/// and every later call is warm.
///
/// Ollama's own default unloads after a few idle minutes, and the reload is not
/// a rounding error — measured on this repo's extraction model, 14.19 s cold
/// against 0.22 s warm. An agent that pauses between calls pays that cliff
/// almost every time, which is precisely the usage pattern here.
///
/// Override with `VELESDB_MEMORY_OLLAMA_KEEP_ALIVE` (any value Ollama accepts,
/// e.g. `30m`, or `0` to unload immediately) when pinning the weights costs
/// more RAM than the latency is worth.
#[cfg(feature = "ollama")]
pub(crate) const DEFAULT_KEEP_ALIVE: i64 = -1;

/// The configured keep-alive as Ollama expects it on the wire.
///
/// The TYPE matters, and getting it wrong fails silently. Ollama reads `-1`
/// (a JSON **number**) as "never unload", but a JSON **string** `"-1"` is not
/// a duration it can parse, so it is dropped and the default 5-minute unload
/// applies — the call looks accepted and the model still disappears. Measured:
/// numeric `-1` yields `expires_at` in year 2318, the string `"-1"` yields
/// five minutes. Duration forms like `30m` are strings and must stay strings.
///
/// So: parse as a number when it is one, pass through as a string otherwise.
#[cfg(feature = "ollama")]
pub(crate) fn keep_alive() -> serde_json::Value {
    let raw = std::env::var("VELESDB_MEMORY_OLLAMA_KEEP_ALIVE")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    match raw {
        None => serde_json::Value::from(DEFAULT_KEEP_ALIVE),
        Some(value) => value.parse::<i64>().map_or_else(
            |_| serde_json::Value::String(value.clone()),
            serde_json::Value::from,
        ),
    }
}

#[cfg(feature = "ollama")]
fn build_request_body(model: &str, text: &str) -> String {
    serde_json::json!({
        "model": model,
        "prompt": text,
        "keep_alive": keep_alive(),
    })
    .to_string()
}

/// Ollama `/api/embeddings` response shape.
#[cfg(feature = "ollama")]
#[derive(Deserialize)]
struct EmbeddingResponse {
    embedding: Vec<f32>,
}

/// Parse an embeddings response body into a vector.
#[cfg(feature = "ollama")]
fn parse_embedding_response(body: &str) -> Result<Vec<f32>, EmbedError> {
    let parsed: EmbeddingResponse = serde_json::from_str(body)
        .map_err(|err| EmbedError::Backend(format!("invalid embeddings response: {err}")))?;
    if parsed.embedding.is_empty() {
        return Err(EmbedError::Empty);
    }
    Ok(parsed.embedding)
}

/// Wall-clock ceiling for one embeddings request.
///
/// Generous enough for a COLD Ollama that has to load the model into memory
/// on the first call (seconds, occasionally tens of seconds), but bounded —
/// which the bare `ureq::post` used before was not. An unbounded wait here is
/// not a slow call, it is a **hung caller**: `remember`/`save_working_context`
/// embed before writing, so an Ollama that accepts the connection and never
/// answers blocks the MCP tool call until the *client* gives up, surfacing as
/// an opaque transport timeout with nothing in the server's own error path.
/// Deliberately far below `extract.rs`'s 300 s: that ceiling covers text
/// GENERATION, while an embedding that has not returned in a minute is not
/// going to.
#[cfg(feature = "ollama")]
const EMBED_TIMEOUT_SECS: u64 = 60;

/// Ceiling on establishing the TCP connection.
///
/// The one setting here that genuinely changes behavior. `ureq` already applies
/// a connect timeout, but its default is 30 s (`agent.rs`) — a sane figure for
/// the open internet and an absurd one for a daemon on `localhost`, which either
/// accepts immediately or is not running. Since retries multiply this wait, 30 s
/// would turn a dead Ollama into a 90 s stall.
#[cfg(feature = "ollama")]
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Ceiling on writing the request. Applied to the socket at connect time, so —
/// unlike the read timeout below — it is in force independently of the global
/// deadline.
#[cfg(feature = "ollama")]
const WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Agent used for every embeddings request, with [`EMBED_TIMEOUT_SECS`]
/// applied. Same pattern as [`crate::extract::OllamaExtractor`], which has
/// bounded its own Ollama calls since it was written.
///
/// # Precedence, stated plainly
///
/// `ureq` documents that `.timeout()` "takes precedence over `.timeout_read()`
/// and `.timeout_write()`, but not `.timeout_connect()`", and its
/// `DeadlineStream` rewrites the socket read deadline to the remaining global
/// budget before every read. So `.timeout_read()` below is **subordinate**: it
/// is declared for the day the global bound is lifted, and must not be read as
/// a per-read ceiling today. `.timeout_connect()` and `.timeout_write()` are the
/// two that bite. Saying otherwise in a doc — or writing a test that claimed to
/// prove a per-read bound — would be a reassurance with nothing behind it.
#[cfg(feature = "ollama")]
fn embed_agent(timeout: std::time::Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_write(WRITE_TIMEOUT)
        .timeout_read(timeout)
        .timeout(timeout)
        .build()
}

/// How one embeddings attempt failed, kept apart just long enough to classify
/// it: transport and body failures may be replayed, a payload the server
/// answered in full is the server's final word.
#[cfg(feature = "ollama")]
enum OllamaCall {
    /// The request never completed (reset, refusal, timeout, HTTP error status).
    /// Boxed: `ureq::Error::Status` carries a whole `Response`.
    Transport(Box<ureq::Error>),
    /// The response headers arrived but the body did not read back in full.
    Body(std::io::Error),
    /// A complete response that is not a usable embedding — deterministic.
    Payload(EmbedError),
}

/// Replay policy for one embeddings attempt.
#[cfg(feature = "ollama")]
fn call_is_retryable(err: &OllamaCall) -> bool {
    match err {
        OllamaCall::Transport(inner) => crate::ollama_retry::is_retryable(inner),
        OllamaCall::Body(inner) => crate::ollama_retry::io_is_retryable(inner),
        OllamaCall::Payload(_) => false,
    }
}

/// The knobs that actually configure this backend, named in its failures.
#[cfg(feature = "ollama")]
const EMBED_LEVERS: crate::ollama_retry::OllamaLevers<'static> =
    crate::ollama_retry::OllamaLevers {
        url_var: "VELESDB_MEMORY_OLLAMA_URL",
        model_var: "VELESDB_MEMORY_OLLAMA_MODEL",
        fallback: Some("fall back to the fully-offline embedder with VELESDB_MEMORY_EMBEDDER=hash"),
    };

/// Perform one embeddings request against a local Ollama, replaying it when the
/// failure is transient.
///
/// The retry is not belt-and-braces. `OllamaEmbedder` holds a single
/// `ureq::Agent`, hence a keep-alive connection pool; Ollama closes idle
/// connections, `ureq` hands the dead one back out, and the POST dies with
/// `Connection reset by peer` — instantly, so the generous `EMBED_TIMEOUT_SECS`
/// never applies. `ureq` will not replay it either: its internal retry demands
/// an idempotent method and an empty body, and this is a POST with a body. The
/// second attempt here dials a fresh connection, which is exactly the repair.
///
/// The whole attempt — POST *and* body read — sits inside the closure, so a
/// truncated response is classified and replayed like any other transport
/// failure instead of surfacing later as an unexplained parse error.
#[cfg(feature = "ollama")]
fn request_embedding(
    agent: &ureq::Agent,
    base_url: &str,
    model: &str,
    text: &str,
) -> Result<Vec<f32>, EmbedError> {
    let url = format!("{base_url}/api/embeddings");
    let body = build_request_body(model, text);
    let attempt = || {
        let response = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|err| OllamaCall::Transport(Box::new(err)))?;
        let payload = response.into_string().map_err(OllamaCall::Body)?;
        parse_embedding_response(&payload).map_err(OllamaCall::Payload)
    };

    match crate::ollama_retry::with_retry(
        &crate::ollama_retry::OLLAMA_RETRIES,
        call_is_retryable,
        attempt,
    ) {
        Ok(vector) => Ok(vector),
        Err((OllamaCall::Payload(err), _)) => Err(err),
        Err((OllamaCall::Transport(err), attempts)) => Err(EmbedError::Backend(
            crate::ollama_retry::actionable_failure(
                "embeddings",
                &url,
                model,
                attempts,
                &err.to_string(),
                &EMBED_LEVERS,
            ),
        )),
        Err((OllamaCall::Body(err), attempts)) => Err(EmbedError::Backend(
            crate::ollama_retry::actionable_failure(
                "embeddings",
                &url,
                model,
                attempts,
                &format!("reading the response failed: {err}"),
                &EMBED_LEVERS,
            ),
        )),
    }
}

#[cfg(all(test, feature = "ollama"))]
#[path = "embedder_tests.rs"]
mod ollama_tests;
