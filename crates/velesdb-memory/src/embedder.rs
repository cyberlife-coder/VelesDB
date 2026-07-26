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

/// Agent used for every embeddings request, with [`EMBED_TIMEOUT_SECS`]
/// applied. Same pattern as [`crate::extract::OllamaExtractor`], which has
/// bounded its own Ollama calls since it was written.
#[cfg(feature = "ollama")]
fn embed_agent(timeout: std::time::Duration) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(timeout).build()
}

/// Perform one blocking embeddings request against a local Ollama.
#[cfg(feature = "ollama")]
fn request_embedding(
    agent: &ureq::Agent,
    base_url: &str,
    model: &str,
    text: &str,
) -> Result<Vec<f32>, EmbedError> {
    let url = format!("{base_url}/api/embeddings");
    let body = build_request_body(model, text);
    let response = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|err| EmbedError::Backend(format!("ollama request failed: {err}")))?;
    let payload = response
        .into_string()
        .map_err(|err| EmbedError::Backend(format!("reading ollama response failed: {err}")))?;
    parse_embedding_response(&payload)
}

#[cfg(all(test, feature = "ollama"))]
mod ollama_tests {
    use super::*;

    #[test]
    fn request_body_carries_model_and_prompt() {
        let body = build_request_body("all-minilm", "hello world");
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(json["model"], "all-minilm");
        assert_eq!(json["prompt"], "hello world");
    }

    #[test]
    fn request_body_pins_the_model_in_memory() {
        // Without `keep_alive` Ollama applies its own default and unloads the
        // model after a few idle minutes, so a call that follows a quiet spell
        // pays a full reload. Measured on this repo's own extraction model:
        // 14.19 s cold against 0.22 s warm — a 64x cliff an agent hits every
        // time it pauses to think.
        let body = build_request_body("all-minilm", "hello world");
        let json: serde_json::Value = serde_json::from_str(&body).expect("valid json");
        assert_eq!(
            json["keep_alive"], DEFAULT_KEEP_ALIVE,
            "request must pin the model for the daemon's lifetime"
        );
        assert!(
            json["keep_alive"].is_number(),
            "Ollama ignores a STRING \"-1\" and unloads after 5 minutes anyway"
        );
    }

    #[test]
    fn parses_a_well_formed_embedding() {
        let vector = parse_embedding_response(r#"{"embedding":[0.1,0.2,0.3]}"#).expect("parse");
        assert_eq!(vector.len(), 3);
        assert!((vector[0] - 0.1_f32).abs() < f32::EPSILON);
    }

    #[test]
    fn rejects_an_empty_embedding() {
        let parsed = parse_embedding_response(r#"{"embedding":[]}"#);
        assert!(matches!(parsed, Err(EmbedError::Empty)));
    }

    #[test]
    fn rejects_a_malformed_response() {
        let parsed = parse_embedding_response(r#"{"oops":true}"#);
        assert!(matches!(parsed, Err(EmbedError::Backend(_))));
    }

    /// An Ollama that accepts the TCP connection and then never answers is
    /// the failure this bound exists for: without it the embed call blocks
    /// forever, and since `remember`/`save_working_context` embed before
    /// writing, the MCP tool call hangs until the CLIENT times out — an
    /// opaque transport error with nothing in the server's own error path.
    /// Uses a 1 s agent so the test stays fast; the shipped ceiling is
    /// `EMBED_TIMEOUT_SECS`.
    #[test]
    fn a_silent_ollama_is_bounded_instead_of_hanging_forever() {
        use std::io::Read as _;
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        // Accept, read the request, then hold the socket open and answer
        // nothing at all — the exact shape of a stalled model load.
        let handle = std::thread::spawn(move || {
            if let Ok((mut socket, _)) = listener.accept() {
                let mut scratch = [0_u8; 1024];
                let _ = socket.read(&mut scratch);
                std::thread::sleep(Duration::from_secs(30));
            }
        });

        let agent = embed_agent(Duration::from_secs(1));
        let started = Instant::now();
        let outcome = request_embedding(&agent, &format!("http://{addr}"), "all-minilm", "hello");
        let elapsed = started.elapsed();

        assert!(
            matches!(outcome, Err(EmbedError::Backend(_))),
            "a silent backend must surface as a Backend error, got {outcome:?}"
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the request must be bounded by the agent timeout, took {elapsed:?}"
        );
        drop(handle);
    }

    #[test]
    fn the_shipped_timeout_stays_bounded_and_usable() {
        // Low enough that an MCP client is still waiting when it fires,
        // high enough to survive a cold model load.
        assert!((5..=120).contains(&EMBED_TIMEOUT_SECS));
    }

    #[test]
    #[ignore = "requires a local Ollama with an embedding model (ollama pull all-minilm)"]
    fn embeds_through_a_running_ollama() {
        let embedder = OllamaEmbedder::new(DEFAULT_OLLAMA_URL, DEFAULT_OLLAMA_MODEL)
            .expect("connect to ollama");
        let vector = embedder
            .embed("parking_lot avoids lock poisoning")
            .expect("embed");
        assert_eq!(vector.len(), embedder.dimension());
        assert!(vector
            .iter()
            .any(|&component| component.abs() > f32::EPSILON));
    }
}
