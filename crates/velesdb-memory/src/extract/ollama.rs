//! The optional `extractor-http` backend: a local Ollama generative model.

use super::prompt::{build_graph_prompt, build_prompt};
use super::reply::{extraction_from_reply, facts_from_reply};
use super::wire::{extraction_schema, fact_list_schema, generate_body, REQUEST_TIMEOUT_SECS};
use super::{ExtractError, ExtractedFact, Extraction, Extractor};

// --- Optional batteries-included backend: a local Ollama generative model -----
//
// Enabled with `--features extractor-http`. A minimal `--no-default-features`
// build omits this backend and its HTTP dependency. Like the Ollama embedder,
// it calls a model the user already runs locally, so the text never leaves the
// machine.

/// Default Ollama base URL for the generative extraction endpoint.
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// The knobs that actually configure the extractor, named in its failures.
///
/// **Not** the embedder's variables. `main.rs`'s `build_ollama_extractor` reads
/// `VELESDB_MEMORY_EXTRACTOR_URL`/`_MODEL`; telling a user to set
/// `VELESDB_MEMORY_OLLAMA_URL` here would send them to edit a setting this code
/// path never consults — an "actionable" message that is actively wrong. There
/// is no offline fallback to offer either: extraction is opt-in, and running
/// without it is simply not passing an extractor.
const EXTRACT_LEVERS: crate::http_retry::FailureLevers<'static> =
    crate::http_retry::FailureLevers {
        url_var: "VELESDB_MEMORY_EXTRACTOR_URL",
        model_var: "VELESDB_MEMORY_EXTRACTOR_MODEL",
        fallback: None,
    };

/// How one generation attempt failed — transport and body failures may be
/// replayed, a complete response is the server's final word.
enum GenerateCall {
    /// The request never completed. Boxed: `ureq::Error::Status` carries a
    /// whole `Response`.
    Transport(Box<ureq::Error>),
    /// Headers arrived but the body did not read back in full.
    Body(std::io::Error),
}

/// Replay policy for one generation attempt.
fn generate_is_retryable(err: &GenerateCall) -> bool {
    match err {
        GenerateCall::Transport(inner) => crate::http_retry::is_retryable(inner),
        GenerateCall::Body(inner) => crate::http_retry::io_is_retryable(inner),
    }
}

/// Turn a failed generation into a message that names the endpoint, the model,
/// how many attempts were spent, and the variables that change the outcome.
fn describe_generate_failure(url: &str, model: &str, err: &GenerateCall, attempts: u32) -> String {
    let cause = match err {
        GenerateCall::Transport(inner) => inner.to_string(),
        GenerateCall::Body(inner) => format!("reading the response failed: {inner}"),
    };
    crate::http_retry::actionable_ollama_failure(
        "generate",
        url,
        model,
        attempts,
        &cause,
        &EXTRACT_LEVERS,
    )
}

/// Extracts facts through a local Ollama `/api/generate` endpoint, keeping the
/// model — and therefore the source text — on the user's own machine.
///
/// The caller picks the generative model (Ollama has no universal default for
/// generation); `temperature` is pinned to `0` and `think` disabled for stable,
/// reproducible output.
#[derive(Debug, Clone)]
pub struct OllamaExtractor {
    base_url: String,
    model: String,
    agent: ureq::Agent,
}

impl OllamaExtractor {
    /// Build an extractor targeting `model` on the Ollama server at `base_url`
    /// (e.g. [`DEFAULT_OLLAMA_URL`]).
    ///
    /// The agent is bounded on four axes, not one. See
    /// [`crate::embedder`]'s `embed_agent` for why `timeout_read` is
    /// subordinate to the global `timeout` in `ureq` and must not be read as a
    /// per-read guarantee; `timeout_connect` and `timeout_write` are the two
    /// that actually bite. The connect bound matters most here: `ureq`'s own
    /// default is 30 s, against the 2 s given here to a `localhost` daemon — and
    /// with replays, that idle wait would be paid three times over.
    #[must_use]
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let agent =
            crate::http_client::bounded_agent(crate::http_client::AgentBudget::local_daemon(
                std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
            ));
        Self {
            base_url: base_url.into(),
            model: model.into(),
            agent,
        }
    }
}

impl Extractor for OllamaExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        facts_from_reply(&self.generate(&build_prompt(text), &fact_list_schema())?)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        extraction_from_reply(&self.generate(&build_graph_prompt(text), &extraction_schema())?)
    }
}

impl OllamaExtractor {
    /// POST one prompt to Ollama's `/api/generate` and return the trimmed reply,
    /// replaying the call when the failure is transient.
    ///
    /// Same defect, same repair as the embedder: this extractor also holds one
    /// `ureq::Agent`, so it also hands out pooled keep-alive connections that
    /// Ollama may have closed, and `ureq` will not replay a POST with a body.
    /// The whole attempt — POST and body read — is inside the closure so a
    /// truncated response is replayed rather than surfacing as a parse error.
    fn generate(&self, prompt: &str, schema: &serde_json::Value) -> Result<String, ExtractError> {
        let url = format!("{}/api/generate", self.base_url);
        let body = generate_body(&self.model, prompt, schema).to_string();
        let attempt = || {
            let response = self
                .agent
                .post(&url)
                .set("Content-Type", "application/json")
                .send_string(&body)
                .map_err(|err| GenerateCall::Transport(Box::new(err)))?;
            response.into_string().map_err(GenerateCall::Body)
        };

        let payload = crate::http_retry::with_retry(
            &crate::http_retry::HTTP_RETRIES,
            generate_is_retryable,
            attempt,
        )
        .map_err(|(err, attempts)| {
            ExtractError::Backend(describe_generate_failure(&url, &self.model, &err, attempts))
        })?;
        parse_generate_response(&payload)
    }
}

/// Pull the `response` string out of Ollama's `/api/generate` JSON envelope.
fn parse_generate_response(body: &str) -> Result<String, ExtractError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| ExtractError::Backend(format!("invalid generate response: {err}")))?;
    let text = value
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ExtractError::Backend("ollama reply had no `response` field".to_string()))?;
    Ok(text.trim().to_string())
}

#[cfg(test)]
#[path = "ollama_tests.rs"]
mod tests;
