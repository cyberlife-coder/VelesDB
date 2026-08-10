//! Tests for the OpenAI-compatible protocol layer: what goes into a request
//! body, and what comes back out of a response.
//!
//! Wire-level authentication is proved elsewhere, in
//! `tests/openai_auth_bdd.rs` — headers belong to the transport, and asserting
//! them here would only prove what this layer *intended*.

// Each test is gated on the feature that keeps its half of the protocol
// alive: CI checks every feature IN ISOLATION, so a test referring to the
// other half would be a compile error there, not merely dead code.
use super::*;

#[test]
#[cfg(feature = "ollama")]
fn an_embeddings_body_carries_the_model_and_the_input() {
    let body = embeddings_body("text-embedding-3-small", "hello world");
    let json: Value = serde_json::from_str(&body).expect("valid json");
    assert_eq!(json["model"], "text-embedding-3-small");
    assert_eq!(json["input"], "hello world");
}

#[test]
#[cfg(feature = "extract")]
fn a_chat_body_pins_temperature_to_zero() {
    // Same reason the Ollama backend pins it: a backend that answers
    // differently to the same text turns one stored fact into two on a re-run.
    let body = chat_body("qwen3", "extract facts", 512);
    let json: Value = serde_json::from_str(&body).expect("valid json");
    assert_eq!(json["model"], "qwen3");
    assert_eq!(json["messages"][0]["role"], "user");
    assert_eq!(json["messages"][0]["content"], "extract facts");
    assert_eq!(json["temperature"], 0);
}

#[test]
#[cfg(feature = "extract")]
fn a_chat_body_caps_completion_tokens() {
    // Unbounded, the real extraction prompt measured 3 933 completion tokens
    // for a twelve-word sentence — 1 min 59 s to store one fact (#1846).
    let body = chat_body("qwen3", "extract facts", 512);
    let json: Value = serde_json::from_str(&body).expect("valid json");
    assert_eq!(json["max_tokens"], 512);
}

#[test]
#[cfg(feature = "ollama")]
fn an_embeddings_response_yields_its_vector() {
    let vector =
        parse_embeddings_response(r#"{"data":[{"embedding":[0.25,-0.5]}]}"#).expect("parsed");
    assert_eq!(vector, vec![0.25, -0.5]);
}

#[test]
#[cfg(feature = "extract")]
fn a_chat_response_yields_the_assistant_message() {
    let content =
        parse_chat_response(r#"{"choices":[{"message":{"content":"[]"}}]}"#).expect("parsed");
    assert_eq!(content, "[]");
}

// --- Negative ----------------------------------------------------------------

#[test]
#[cfg(feature = "ollama")]
fn an_error_envelope_is_reported_as_a_refusal_not_a_malformed_response() {
    // A server that answers 200 with `{"error":{...}}` is common enough that
    // reading past the envelope would report "no data[0].embedding" for a
    // perfectly clear refusal, sending the reader to look for the wrong bug.
    let err = parse_embeddings_response(r#"{"error":{"message":"model not found"}}"#)
        .expect_err("an error envelope is not a vector");
    assert!(
        err.contains("model not found"),
        "the server's own words must survive into the message, got: {err}"
    );
    assert!(
        err.contains("refused"),
        "and it must read as a refusal, got: {err}"
    );
}

#[test]
#[cfg(feature = "extract")]
fn a_non_json_response_names_what_came_back() {
    let err = parse_chat_response("<html>502 Bad Gateway</html>")
        .expect_err("HTML is not a chat completion");
    assert!(
        err.contains("not JSON") && err.contains("502"),
        "the reader must see what actually arrived, got: {err}"
    );
}

#[test]
#[cfg(feature = "ollama")]
fn a_missing_field_is_reported_rather_than_silently_empty() {
    let err = parse_embeddings_response(r#"{"data":[]}"#)
        .expect_err("an empty data array carries no vector");
    assert!(
        err.contains("data[0].embedding"),
        "the message must name the field it looked for, got: {err}"
    );
}

#[test]
#[cfg(feature = "extract")]
fn a_long_payload_is_previewed_not_pasted_whole() {
    let payload = format!(r#"{{"junk":"{}"}}"#, "x".repeat(5_000));
    let err = parse_chat_response(&payload).expect_err("no content field");
    assert!(
        err.len() < 400,
        "a whole model response must not land in an error message, got {} chars",
        err.len()
    );
    assert!(err.contains('…'), "the preview must say it was cut: {err}");
}

// --- The base URL an operator actually copies --------------------------------

#[test]
fn a_base_url_carrying_the_version_prefix_is_not_doubled() {
    // The exact string oMLX's console shows beside a copy button. Left as-is,
    // it would produce `/v1/v1/embeddings` and a 404 the operator cannot
    // explain — they pasted the vendor's own URL.
    assert_eq!(
        base_url("http://127.0.0.1:8019/v1"),
        "http://127.0.0.1:8019"
    );
    assert_eq!(
        base_url("http://127.0.0.1:8019/v1/"),
        "http://127.0.0.1:8019",
        "a trailing slash on the copied URL must not defeat it either"
    );
}

#[test]
fn a_bare_origin_is_left_alone() {
    assert_eq!(base_url("http://localhost:8020"), "http://localhost:8020");
    assert_eq!(
        base_url("  http://localhost:8020/  "),
        "http://localhost:8020",
        "surrounding whitespace comes free with a shell variable"
    );
}

#[test]
fn a_server_mounted_under_a_path_keeps_its_path() {
    assert_eq!(
        base_url("https://gateway.example/models/v1"),
        "https://gateway.example/models",
        "stripping the protocol's own prefix must not eat the mount point"
    );
}

#[test]
fn a_host_genuinely_named_v1_is_not_truncated() {
    // Edge case with a sharp failure mode: a blind `strip_suffix("/v1")` turns
    // `http://v1` into `http:/`, which fails as a malformed URL rather than as
    // an unreachable host.
    assert_eq!(base_url("http://v1"), "http://v1");
}
