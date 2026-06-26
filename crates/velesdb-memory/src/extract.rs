//! Optional text → facts + entities extraction, the layer that makes the graph
//! self-build.
//!
//! The Agent Memory SDK is *bring-your-own-links*: [`crate::MemoryService::remember`]
//! only stores the links the caller supplies, so a graph is only ever as rich as
//! what the caller wires by hand. This module adds the missing commodity on top:
//! an [`Extractor`] turns a paragraph of raw text into atomic facts, each tagged
//! with the salient topics it mentions. [`crate::MemoryService::remember_extracted`]
//! then stores those facts and wires the fact↔entity graph automatically, so
//! `why()` has something to traverse without any manual `relate()`.
//!
//! Mirroring the [`crate::embedder`] pattern, the plug-point is dependency-free
//! (bring your own LLM by implementing [`Extractor`]) while a batteries-included
//! [`OllamaExtractor`] backend lives behind the `extract` feature.

/// One extracted, graph-ready fact: a self-contained sentence plus the salient
/// topics it concerns. The topics become shared graph hubs, so two facts about
/// the same topic are reachable from one another even with no textual overlap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedFact {
    /// The atomic, standalone fact (pronouns resolved, dates absolute).
    pub text: String,
    /// Salient topics the fact concerns — short canonical lowercase noun
    /// phrases (e.g. `"adoption"`, `"charity race"`). 1-4 is typical.
    pub entities: Vec<String>,
}

/// Failure produced by an [`Extractor`] backend (e.g. a network-backed model
/// that cannot be reached, or output that cannot be parsed into facts).
#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    /// The extraction backend (network, subprocess, …) returned an error.
    #[error("extraction backend error: {0}")]
    Backend(String),
    /// The backend produced output that could not be parsed into facts.
    #[error("could not parse facts from extractor output: {0}")]
    Parse(String),
}

/// Turns a passage of raw text into atomic, graph-ready facts.
///
/// Implement this to plug in any model — a local LLM, a hosted API, or a
/// deterministic rule set — and feed the result straight into
/// [`crate::MemoryService::remember_extracted`].
pub trait Extractor {
    /// Extract the atomic facts a reader would remember from `text`.
    ///
    /// # Errors
    /// Returns [`ExtractError`] if the backend fails or its output cannot be
    /// parsed into facts.
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError>;
}

// --- Optional batteries-included backend: a local Ollama generative model -----
//
// Enabled with `--features extract`. The default build omits this backend (and
// its HTTP dependency) so the shipped binary stays tiny and fully offline. Like
// the Ollama embedder, it calls a model the user already runs locally, so the
// text never leaves the machine.

/// Default Ollama base URL for the generative extraction endpoint.
#[cfg(feature = "extract")]
pub const DEFAULT_OLLAMA_URL: &str = "http://localhost:11434";

/// Per-request timeout. Generation is far slower and more stall-prone than an
/// embedding call, so a wedged model fails the call instead of hanging forever.
#[cfg(feature = "extract")]
const REQUEST_TIMEOUT_SECS: u64 = 300;

/// Extracts facts through a local Ollama `/api/generate` endpoint, keeping the
/// model — and therefore the source text — on the user's own machine.
///
/// The caller picks the generative model (Ollama has no universal default for
/// generation); `temperature` is pinned to `0` and `think` disabled for stable,
/// reproducible output.
#[cfg(feature = "extract")]
#[derive(Debug, Clone)]
pub struct OllamaExtractor {
    base_url: String,
    model: String,
    agent: ureq::Agent,
}

#[cfg(feature = "extract")]
impl OllamaExtractor {
    /// Build an extractor targeting `model` on the Ollama server at `base_url`
    /// (e.g. [`DEFAULT_OLLAMA_URL`]).
    #[must_use]
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .build();
        Self {
            base_url: base_url.into(),
            model: model.into(),
            agent,
        }
    }
}

#[cfg(feature = "extract")]
impl Extractor for OllamaExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        let reply = self.generate(&build_prompt(text))?;
        let raw = json_slice::<Vec<RawFact>>(&reply)
            .ok_or_else(|| ExtractError::Parse(truncate(&reply)))?;
        Ok(raw.into_iter().filter_map(RawFact::into_fact).collect())
    }
}

#[cfg(feature = "extract")]
impl OllamaExtractor {
    /// POST one prompt to Ollama's `/api/generate` and return the trimmed reply.
    fn generate(&self, prompt: &str) -> Result<String, ExtractError> {
        let url = format!("{}/api/generate", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false,
            "think": false,
            "options": { "temperature": 0 },
        })
        .to_string();
        let response = self
            .agent
            .post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body)
            .map_err(|err| ExtractError::Backend(format!("ollama request failed: {err}")))?;
        let payload = response.into_string().map_err(|err| {
            ExtractError::Backend(format!("reading ollama response failed: {err}"))
        })?;
        parse_generate_response(&payload)
    }
}

/// The strict JSON contract the extraction prompt asks the model to honour.
#[cfg(feature = "extract")]
#[derive(serde::Deserialize)]
struct RawFact {
    fact: String,
    #[serde(default)]
    entities: Vec<String>,
}

#[cfg(feature = "extract")]
impl RawFact {
    /// Keep a fact only if it has text; trim and lowercase its topics, dropping
    /// blanks and duplicates so the same topic recurs as the same graph hub.
    fn into_fact(self) -> Option<ExtractedFact> {
        let text = self.fact.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let mut seen = std::collections::HashSet::new();
        let entities = self
            .entities
            .into_iter()
            .map(|entity| entity.trim().to_lowercase())
            .filter(|entity| !entity.is_empty() && seen.insert(entity.clone()))
            .collect();
        Some(ExtractedFact { text, entities })
    }
}

/// Build the extraction prompt: the passage plus a strict JSON contract.
#[cfg(feature = "extract")]
fn build_prompt(text: &str) -> String {
    format!(
        "You are building a memory graph from the passage below.\n\n\
Passage:\n{text}\n\n\
Extract the atomic, standalone facts a person would remember. Rewrite each as a \
self-contained sentence (resolve pronouns to names; keep absolute dates). For \
each fact also list 1-4 key TOPICS it concerns: the recurring subjects, \
activities, events, interests, plans, places, organisations, or named people a \
later question might reference. Use short, canonical, lowercase noun phrases \
(e.g. \"adoption\", \"charity race\", \"therapy\", \"new job\") so the same topic \
recurs as the SAME tag across passages.\n\n\
Return ONLY a JSON array, no prose, each item exactly:\n\
{{\"fact\": string, \"entities\": [string]}}"
    )
}

/// Pull the `response` string out of Ollama's `/api/generate` JSON envelope.
#[cfg(feature = "extract")]
fn parse_generate_response(body: &str) -> Result<String, ExtractError> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|err| ExtractError::Backend(format!("invalid generate response: {err}")))?;
    let text = value
        .get("response")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ExtractError::Backend("ollama reply had no `response` field".to_string()))?;
    Ok(text.trim().to_string())
}

/// A short, single-line preview of model output for error messages.
#[cfg(feature = "extract")]
fn truncate(text: &str) -> String {
    let oneline = text.split_whitespace().collect::<Vec<_>>().join(" ");
    oneline.chars().take(120).collect()
}

/// Parse `text` into `T`, first slicing out the outermost JSON array/object.
/// Local models usually honour "return only JSON" but occasionally wrap it in
/// fences or a sentence; slicing the first balanced span tolerates that.
#[cfg(feature = "extract")]
fn json_slice<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice(text)?;
    serde_json::from_str::<T>(slice).ok()
}

/// Return the substring spanning the first balanced `[..]` or `{..}`, honouring
/// string literals and escapes so brackets inside quotes don't miscount.
#[cfg(feature = "extract")]
fn balanced_slice(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'[' || b == b'{')?;
    let open = bytes[start];
    let close = if open == b'[' { b']' } else { b'}' };
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &byte) in bytes[start..].iter().enumerate() {
        if in_string {
            in_string = step_string(&mut escaped, byte);
        } else if scan_structural(byte, open, close, &mut in_string, &mut depth) {
            return Some(&text[start..=start + offset]);
        }
    }
    None
}

/// Advance the structural scan for one out-of-string byte; returns `true` once
/// the outermost bracket has just closed (`depth` back to zero).
#[cfg(feature = "extract")]
fn scan_structural(byte: u8, open: u8, close: u8, in_string: &mut bool, depth: &mut u32) -> bool {
    if byte == b'"' {
        *in_string = true;
    } else if byte == open {
        *depth += 1;
    } else if byte == close {
        *depth = depth.saturating_sub(1);
        return *depth == 0;
    }
    false
}

/// Advance the in-string escape state for one byte; returns whether the scanner
/// is still inside the string literal afterwards.
#[cfg(feature = "extract")]
fn step_string(escaped: &mut bool, byte: u8) -> bool {
    match (*escaped, byte) {
        (true, _) => {
            *escaped = false;
            true
        }
        (false, b'\\') => {
            *escaped = true;
            true
        }
        (false, b'"') => false,
        (false, _) => true,
    }
}

#[cfg(all(test, feature = "extract"))]
mod tests {
    use super::*;

    #[test]
    fn prompt_carries_the_passage_and_json_contract() {
        let prompt = build_prompt("Alice adopted a dog in 2021.");
        assert!(prompt.contains("Alice adopted a dog in 2021."));
        assert!(prompt.contains("\"fact\": string"));
    }

    #[test]
    fn parses_facts_from_a_fenced_reply() {
        let reply = "Sure!\n```json\n[{\"fact\":\"Alice adopted a dog.\",\"entities\":[\"Adoption\",\"adoption\",\"\"]}]\n```";
        let facts: Vec<RawFact> = json_slice(reply).expect("slice json");
        let facts: Vec<ExtractedFact> = facts.into_iter().filter_map(RawFact::into_fact).collect();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].text, "Alice adopted a dog.");
        // Trimmed, lowercased, deduplicated, blanks dropped.
        assert_eq!(facts[0].entities, vec!["adoption".to_string()]);
    }

    #[test]
    fn drops_a_textless_fact() {
        let raw = RawFact {
            fact: "   ".to_string(),
            entities: vec!["x".to_string()],
        };
        assert!(raw.into_fact().is_none());
    }

    #[test]
    fn parses_response_envelope() {
        let text = parse_generate_response(r#"{"response":"  [] "}"#).expect("parse");
        assert_eq!(text, "[]");
    }

    #[test]
    fn rejects_response_without_field() {
        assert!(matches!(
            parse_generate_response(r#"{"oops":true}"#),
            Err(ExtractError::Backend(_))
        ));
    }
}
