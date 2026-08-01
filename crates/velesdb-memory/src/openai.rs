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
#[cfg(any(feature = "ollama", feature = "extract"))]
pub(crate) fn base_url(configured: &str) -> String {
    let trimmed = configured.trim().trim_end_matches('/');
    match trimmed.strip_suffix("/v1") {
        Some(base) if !base.is_empty() && !base.ends_with('/') => base.to_owned(),
        _ => trimmed.to_owned(),
    }
}

/// Embeddings endpoint, relative to the caller's base URL.
#[cfg(feature = "ollama")]
pub(crate) const EMBEDDINGS_PATH: &str = "/v1/embeddings";

/// Chat-completions endpoint, relative to the caller's base URL.
#[cfg(feature = "extract")]
pub(crate) const CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

/// Body of an embeddings request.
#[cfg(feature = "ollama")]
pub(crate) fn embeddings_body(model: &str, input: &str) -> String {
    json!({ "model": model, "input": input }).to_string()
}

/// Body of a single-turn chat-completions request.
///
/// `temperature: 0` for the same reason the Ollama backend pins it: a backend
/// that answers differently to the same text turns one stored fact into two
/// on a re-run.
#[cfg(feature = "extract")]
pub(crate) fn chat_body(model: &str, prompt: &str) -> String {
    json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }],
        "temperature": 0,
    })
    .to_string()
}

/// The vector out of an embeddings response: `{"data":[{"embedding":[...]}]}`.
///
/// # Errors
/// A message naming what was wrong with the payload, including the protocol's
/// own `{"error":{"message":...}}` envelope when the server sent one — a
/// server that answers `200` with an error body is common enough that reading
/// past it would report "malformed response" for a perfectly clear refusal.
#[cfg(feature = "ollama")]
pub(crate) fn parse_embeddings_response(payload: &str) -> Result<Vec<f32>, String> {
    let value = parse_json(payload)?;
    // Deserialized into a typed `Vec<f32>` rather than walked as `Value` and
    // cast: serde narrows each component, so there is no hand-written
    // `as f32` to justify — the same shape the Ollama backend already uses.
    let response: EmbeddingsResponse = serde_json::from_value(value).map_err(|err| {
        format!(
            "response has no `data[0].embedding` array ({err}): {}",
            preview(payload)
        )
    })?;
    response
        .data
        .into_iter()
        .next()
        .map(|datum| datum.embedding)
        .ok_or_else(|| {
            format!(
                "response has no `data[0].embedding` array: {}",
                preview(payload)
            )
        })
}

/// `{"data":[{"embedding":[...]}]}` — only the field this crate reads.
#[cfg(feature = "ollama")]
#[derive(serde::Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingDatum>,
}

#[cfg(feature = "ollama")]
#[derive(serde::Deserialize)]
struct EmbeddingDatum {
    embedding: Vec<f32>,
}

/// The assistant message out of a chat-completions response:
/// `{"choices":[{"message":{"content":"..."}}]}`.
///
/// # Errors
/// As [`parse_embeddings_response`].
#[cfg(feature = "extract")]
pub(crate) fn parse_chat_response(payload: &str) -> Result<String, String> {
    let value = parse_json(payload)?;
    value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|first| first.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "response has no `choices[0].message.content` string: {}",
                preview(payload)
            )
        })
}

/// Parse a payload, surfacing the protocol's error envelope as the message
/// when there is one.
fn parse_json(payload: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(payload)
        .map_err(|err| format!("response is not JSON ({err}): {}", preview(payload)))?;
    if let Some(message) = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
    {
        return Err(format!("the server refused the request: {message}"));
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
