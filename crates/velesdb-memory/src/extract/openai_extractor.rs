//! The `OpenAI`-compatible extraction backend. It shares the prompts, the reply
//! reader and the request limits with the Ollama backend: only the transport
//! differs between the two.

use super::prompt::{build_graph_prompt, build_prompt};
use super::reply::{extraction_from_reply, facts_from_reply};
use super::wire::{MAX_GENERATION_TOKENS, REQUEST_TIMEOUT_SECS};
use super::{ExtractError, ExtractedFact, Extraction, Extractor};

/// Extracts through any **OpenAI-compatible** `/v1/chat/completions` endpoint.
///
/// A sibling of [`OllamaExtractor`](super::OllamaExtractor), not a layer over it — the same shape the
/// embedding role takes. The prompt stays here, on the role side: it is what
/// this crate wants said, not something the protocol knows about.
pub struct OpenAiExtractor {
    client: crate::http_client::HttpJsonClient,
    model: String,
    chat_template_kwargs: Option<serde_json::Value>,
}

/// Hand-written for the reason [`crate::http_client::Auth`]'s is: the template
/// arguments are operator-supplied, a chat template can read any key it is
/// given, and a derive would print them into whatever log or panic message
/// carries this struct. The field stays listed — only its value is withheld —
/// so a `Debug` dump still tells whether the setting is in effect.
impl std::fmt::Debug for OpenAiExtractor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiExtractor")
            .field("client", &self.client)
            .field("model", &self.model)
            .field(
                "chat_template_kwargs",
                &self.chat_template_kwargs.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl OpenAiExtractor {
    /// Build an extractor targeting `model` on the server at `base_url`
    /// (origin and port, no path).
    ///
    /// Bounded on the same four axes as [`OllamaExtractor::new`](super::OllamaExtractor::new), with the
    /// same generous `REQUEST_TIMEOUT_SECS`: generation is slow wherever it
    /// runs, and the ceiling belongs to the role, not to the transport.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        auth: crate::http_client::Auth,
    ) -> Self {
        let agent =
            crate::http_client::bounded_agent(crate::http_client::AgentBudget::local_daemon(
                std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
            ));
        Self {
            client: crate::http_client::HttpJsonClient::new(
                crate::openai::base_url(&base_url.into()),
                auth,
                agent,
            ),
            model: model.into(),
            chat_template_kwargs: None,
        }
    }

    /// Send `kwargs` as the chat body's `chat_template_kwargs` on every call.
    ///
    /// llama.cpp-compatible servers read it as per-request chat-template
    /// arguments — e.g. `{"enable_thinking": false}` for a thinking model,
    /// whose thinking tokens would otherwise eat the generation budget and
    /// truncate the JSON this role parses. Servers that do not know the field
    /// ignore it. Without this call the field is absent from the body.
    ///
    /// `kwargs` is sent verbatim and should be a JSON object, which is what
    /// the field means; the daemon enforces that when it reads
    /// `VELESDB_MEMORY_EXTRACTOR_CHAT_TEMPLATE_KWARGS`, and this library does
    /// not read the environment itself. The value is left out of the
    /// [`Debug`] output.
    #[must_use]
    pub fn with_chat_template_kwargs(mut self, kwargs: serde_json::Value) -> Self {
        self.chat_template_kwargs = Some(kwargs);
        self
    }

    /// POST one prompt and return the assistant's reply.
    fn generate(&self, prompt: &str) -> Result<String, ExtractError> {
        let body = crate::openai::chat_body(
            &self.model,
            prompt,
            MAX_GENERATION_TOKENS,
            self.chat_template_kwargs.as_ref(),
        );
        let payload = self
            .client
            .post_json(crate::openai::CHAT_COMPLETIONS_PATH, &body)
            .map_err(|failure| {
                ExtractError::Backend(crate::http_retry::actionable_openai_failure(
                    "chat/completions",
                    &failure.url,
                    &self.model,
                    failure.attempts,
                    &failure.cause,
                    Some(
                        "use the offline deterministic reader with \
                         VELESDB_MEMORY_EXTRACTOR=outline",
                    ),
                ))
            })?;
        crate::openai::parse_chat_response(&payload)
            .map_err(|error| ExtractError::Backend(error.to_string()))
    }
}

impl Extractor for OpenAiExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        facts_from_reply(&self.generate(&build_prompt(text))?)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        extraction_from_reply(&self.generate(&build_graph_prompt(text))?)
    }
}

#[cfg(test)]
#[path = "openai_extractor_tests.rs"]
mod tests;
