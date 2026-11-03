//! The OpenAI-compatible protocol: which paths exist, what a request body
//! looks like, and how to read an answer back.
//!
//! Sits between [`crate::http_client`] (which knows only how to post JSON with
//! a credential) and the two backends that use it. It knows both endpoints of
//! the protocol because the protocol has both — that is not role logic, no
//! more than an HTTP library knowing about `GET` and `POST` is. What it does
//! NOT know: the [`crate::Embedder`] trait, dimension probing, extraction
//! prompts, or anything else that belongs to a caller.
//!
//! "OpenAI-compatible" is a protocol, not a vendor. oMLX, llama.cpp's server,
//! LM Studio, vLLM and the hosted providers all speak it, and reaching a new
//! one means a different base URL — never a new value to add here.

use serde_json::{json, Value};

/// The base URL to concatenate this protocol's paths onto, from whatever the
/// operator configured.
///
/// **Both spellings of the same endpoint are accepted**, and that is not
/// leniency for its own sake: servers advertise their OpenAI-compatible
/// endpoint WITH the version prefix. oMLX's own console shows
/// `http://127.0.0.1:8019/v1` beside a copy button, and LM Studio and vLLM do
/// the same. Concatenating [`EMBEDDINGS_PATH`] onto a copied URL would produce
/// `/v1/v1/embeddings` and a `404` whose cause is invisible from the message —
/// the operator sees a URL they copied from the vendor's own UI being refused.
///
/// The `/v1` prefix belongs to this protocol, so recognising it in the base
/// URL belongs here too, and nowhere else: [`crate::http_client`] must keep
/// concatenating a path onto a string it knows nothing about.
///
/// A trailing `/v1` is stripped only when something precedes it, so a host
/// genuinely called `v1` (`http://v1`) is left alone.
#[cfg(any(feature = "embedder-http", feature = "extractor-http"))]
pub(crate) fn base_url(configured: &str) -> String {
    let trimmed = configured.trim().trim_end_matches('/');
    match trimmed.strip_suffix("/v1") {
        Some(base) if !base.is_empty() && !base.ends_with('/') => base.to_owned(),
        _ => trimmed.to_owned(),
    }
}

/// Embeddings endpoint, relative to the caller's base URL.
#[cfg(feature = "embedder-http")]
pub(crate) const EMBEDDINGS_PATH: &str = "/v1/embeddings";

/// Chat-completions endpoint, relative to the caller's base URL.
#[cfg(feature = "extractor-http")]
pub(crate) const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// Body of an embeddings request.
#[cfg(feature = "embedder-http")]
pub(crate) fn embeddings_body(model: &str, input: &str) -> String {
    json!({ "model": model, "input": input }).to_string()
}

/// Body of a single-turn chat-completions request.
///
/// `temperature: 0` for the same reason the Ollama backend pins it: a backend
/// that answers differently to the same text turns one stored fact into two
/// on a re-run. `max_tokens` caps runaway generation the same way the Ollama
/// backend's `num_predict` does — see `extract::MAX_GENERATION_TOKENS` (#1846).
///
/// `json_schema` restates the prompt's reply contract as a grammar the
/// sampler cannot leave — the OpenAI-protocol spelling (`response_format`,
/// `strict`) of the `format` the Ollama backend sends for the same reason
/// (#1944): it makes replies parsable, it does not make them true. `strict`
/// is the arm the GitHub Models probe measured; it requires every object
/// level closed, which [`closed_for_strict`] adds at this protocol edge
/// rather than in the canonical schema, where Ollama's grammar needs none.
///
/// `chat_template_kwargs` is passed through verbatim when set (llama.cpp's
/// per-request chat-template knobs, e.g. `{"enable_thinking": false}`);
/// servers that do not know the field ignore it.
#[cfg(feature = "extractor-http")]
pub(crate) fn chat_body(
    model: &str,
    prompt: &str,
    max_tokens: u32,
    json_schema: &Value,
    chat_template_kwargs: Option<&Value>,
) -> String {
    let mut body = json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0,
        "max_tokens": max_tokens,
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "extraction",
                "strict": true,
                "schema": closed_for_strict(json_schema),
            },
        },
    });
    if let Some(kwargs) = chat_template_kwargs {
        body["chat_template_kwargs"] = kwargs.clone();
    }
    body.to_string()
}

/// `schema` with `additionalProperties: false` at every `"type": "object"`
/// level — the strict-mode spelling of the same contract.
///
/// Strict `json_schema` refuses a schema whose object levels leave
/// `additionalProperties` open (a 400, not a silent fallback). Same keys,
/// same `required`s — one mechanical clause added per object level, the
/// transform `strict_object_schema` in `scripts/bench-memory-extraction.py`
/// applied on the probe arm that decided this wiring.
#[cfg(feature = "extractor-http")]
fn closed_for_strict(schema: &Value) -> Value {
    let mut closed = schema.clone();
    close_object_level(&mut closed);
    closed
}

/// Recursive half of [`closed_for_strict`]: non-object values are left alone.
#[cfg(feature = "extractor-http")]
fn close_object_level(node: &mut Value) {
    let Some(map) = node.as_object_mut() else {
        return;
    };
    if map.get("type") == Some(&Value::from("object")) {
        map.insert("additionalProperties".to_owned(), Value::from(false));
    }
    for value in map.values_mut() {
        close_object_level(value);
    }
}

/// Why an OpenAI-compatible answer could not be read. The payload is previewed
/// ([`preview`]), never pasted whole, so a message stays short.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ResponseError {
    /// The body is not JSON at all (an HTML gateway page, a truncated stream).
    #[error("response is not JSON ({cause}): {preview}")]
    NotJson {
        cause: serde_json::Error,
        preview: String,
    },
    /// The server answered `200` with the protocol's own
    /// `{"error":{"message":...}}` envelope.
    #[error("the server refused the request: {message}")]
    Refused { message: String },
    /// The body is JSON but does not have the embeddings shape.
    #[cfg(feature = "embedder-http")]
    #[error("response has no `data[0].embedding` array ({cause}): {preview}")]
    MalformedEmbeddings {
        cause: serde_json::Error,
        preview: String,
    },
    /// The embeddings shape is right but `data` is empty.
    #[cfg(feature = "embedder-http")]
    #[error("response has no `data[0].embedding` array: {preview}")]
    NoEmbedding { preview: String },
    /// The chat completion carries no assistant text.
    #[cfg(feature = "extractor-http")]
    #[error("response has no `choices[0].message.content` string: {preview}")]
    NoContent { preview: String },
}

/// The vector out of an embeddings response: `{"data":[{"embedding":[...]}]}`.
///
/// # Errors
/// [`ResponseError::Refused`] when the server sent the protocol's own
/// `{"error":{"message":...}}` envelope — a server that answers `200` with an
/// error body is common enough that reading past it would report "malformed
/// response" for a perfectly clear refusal — otherwise the variant naming what
/// was wrong with the payload.
#[cfg(feature = "embedder-http")]
pub(crate) fn parse_embeddings_response(payload: &str) -> Result<Vec<f32>, ResponseError> {
    let value = parse_json(payload)?;
    // Deserialized into a typed `Vec<f32>` rather than walked as `Value` and
    // cast: serde narrows each component, so there is no hand-written
    // `as f32` to justify — the same shape the Ollama backend already uses.
    let response: EmbeddingsResponse =
        serde_json::from_value(value).map_err(|cause| ResponseError::MalformedEmbeddings {
            cause,
            preview: preview(payload),
        })?;
    response
        .data
        .into_iter()
        .next()
        .map(|datum| datum.embedding)
        .ok_or_else(|| ResponseError::NoEmbedding {
            preview: preview(payload),
        })
}

/// `{"data":[{"embedding":[...]}]}` — only the field this crate reads.
#[cfg(feature = "embedder-http")]
#[derive(serde::Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingDatum>,
}

#[cfg(feature = "embedder-http")]
#[derive(serde::Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

/// The assistant message out of a chat-completions response:
/// `{"choices":[{"message":{"content":"..."}}]}`.
///
/// # Errors
/// As [`parse_embeddings_response`].
#[cfg(feature = "extractor-http")]
pub(crate) fn parse_chat_response(payload: &str) -> Result<String, ResponseError> {
    let value = parse_json(payload)?;
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|first| first.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ResponseError::NoContent {
            preview: preview(payload),
        })
}

/// Parse a payload, surfacing the protocol's error envelope as a refusal
/// when there is one.
fn parse_json(payload: &str) -> Result<Value, ResponseError> {
    let value: Value = serde_json::from_str(payload).map_err(|cause| ResponseError::NotJson {
        cause,
        preview: preview(payload),
    })?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return Err(ResponseError::Refused {
            message: message.to_owned(),
        });
    }
    Ok(value)
}

/// First 200 bytes of a payload, on a char boundary — enough to recognise what
/// came back without pasting a whole model response into an error.
fn preview(payload: &str) -> String {
    let cut = payload
        .char_indices()
        .map(|(at, _)| at)
        .take_while(|at| *at <= 200)
        .last()
        .unwrap_or(0);
    if cut < payload.len() {
        format!("{}…", &payload[..cut])
    } else {
        payload.to_owned()
    }
}

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
