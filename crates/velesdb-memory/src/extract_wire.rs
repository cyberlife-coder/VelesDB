//! The wire contract of one extraction call: the request body sent to Ollama,
//! and the JSON Schemas sent inside it.
//!
//! Split out of `extract.rs` when constraining decoding (#1944) pushed that
//! file past its frozen budget. The seam is how ONE backend is asked: Ollama's
//! request envelope and the schemas that constrain its decoding. It is not
//! everything that leaves the process — the prompts leave too, and stay in
//! `extract.rs` beside the parser, because they are what this crate wants said
//! to any backend: the OpenAI-compatible extractor sends the same prompts, in
//! an envelope `crate::openai` builds, and no schema.
//!
//! Each schema still sits beside nothing else, which is the point — it is one
//! contract stated twice, once for the sampler and once for the reader, and
//! `every_required_key_of_the_schema_is_a_key_the_parser_reads` is what keeps
//! the two from drifting.

use super::MAX_GENERATION_TOKENS;

/// The body of one `/api/generate` call, built where a test can read it.
///
/// Split out of [`super::OllamaExtractor`]'s `generate` so the wire contract can be
/// asserted without a live model: a `format` that silently stopped being sent
/// would restore the exact defect this shape exists to close, and no
/// stub-backed extraction test would notice.
#[cfg(feature = "extractor-http")]
pub(super) fn generate_body(
    model: &str,
    prompt: &str,
    schema: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "prompt": prompt,
        "stream": false,
        "think": false,
        // The same contract the prompt states, restated as a grammar the
        // sampler cannot leave. Structure only: it makes replies parsable, it
        // does not make them true, and the prompt stays the place that says
        // what to extract.
        "format": schema,
        // Extraction models are large — the one this crate documents as an
        // example is 21.9 GB — so an unload between calls is the dominant
        // cost, not the generation. Shares the embedder's knob so one setting
        // governs every Ollama call the daemon makes.
        "keep_alive": crate::embedder::keep_alive(),
        "options": { "temperature": 0, "num_predict": MAX_GENERATION_TOKENS },
    })
}

/// One `{"fact": …, "entities": […]}` item, as a JSON Schema.
///
/// Kept beside [`super::RawFact`], which is what parses it back: the two are one
/// contract stated twice, once for the sampler and once for the reader, and
/// they must be changed together.
#[cfg(feature = "extractor-http")]
pub(super) fn fact_item_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "fact": { "type": "string" },
            "entities": { "type": "array", "items": { "type": "string" } },
        },
        "required": ["fact", "entities"],
    })
}

/// The JSON Schema for [`super::build_prompt`]'s reply: a bare array of facts.
#[cfg(feature = "extractor-http")]
pub(super) fn fact_list_schema() -> serde_json::Value {
    serde_json::json!({ "type": "array", "items": fact_item_schema() })
}

/// The JSON Schema for [`super::build_graph_prompt`]'s reply, mirroring
/// [`super::RawExtraction`].
///
/// `value` keeps the `number` arm the prompt insists on ("Emit numbers as JSON
/// NUMBERS, never strings"): constraining it to `string` would silently undo
/// what the prompt spends a line asking for.
#[cfg(feature = "extractor-http")]
pub(super) fn extraction_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "facts": { "type": "array", "items": fact_item_schema() },
            "relations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string" },
                        "predicate": { "type": "string" },
                        "object": { "type": "string" },
                    },
                    "required": ["subject", "predicate", "object"],
                },
            },
            "attributes": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "entity": { "type": "string" },
                        "key": { "type": "string" },
                        "value": { "type": ["string", "number", "boolean"] },
                    },
                    "required": ["entity", "key", "value"],
                },
            },
        },
        "required": ["facts", "relations", "attributes"],
    })
}
