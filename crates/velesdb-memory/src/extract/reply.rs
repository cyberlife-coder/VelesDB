//! Reading a generative model's reply: the strict JSON contract and the slicing
//! that tolerates the prose and fences a local model sometimes wraps it in.
//!
//! Shared by every generative backend: only the transport differs between them.

use super::{ExtractError, ExtractedAttribute, ExtractedFact, ExtractedRelation, Extraction};

/// Read a model's reply as the flat fact list [`super::Extractor::extract`] promises.
///
/// A free function because every generative backend produces the same reply
/// and reads it the same way — only the transport differs. Leaving a copy in
/// each `impl` would let two backends drift on what counts as a valid answer.
pub(super) fn facts_from_reply(reply: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
    let raw =
        json_slice::<Vec<RawFact>>(reply).ok_or_else(|| ExtractError::Parse(truncate(reply)))?;
    Ok(raw.into_iter().filter_map(RawFact::into_fact).collect())
}

/// [`facts_from_reply`]'s counterpart for [`super::Extractor::extract_graph`].
pub(super) fn extraction_from_reply(reply: &str) -> Result<Extraction, ExtractError> {
    let raw = json_slice_object::<RawExtraction>(reply)
        .ok_or_else(|| ExtractError::Parse(truncate(reply)))?;
    Ok(raw.into_extraction())
}

/// The strict JSON contract the extraction prompt asks the model to honour.
#[derive(serde::Deserialize)]
struct RawFact {
    fact: String,
    #[serde(default)]
    entities: Vec<String>,
}

impl RawFact {
    /// Keep a fact only if it has text; trim and lowercase its topics, dropping
    /// blanks and duplicates so the same topic recurs as the same graph hub.
    fn into_fact(self) -> Option<ExtractedFact> {
        let text = self.fact.trim().to_string();
        if text.is_empty() {
            return None;
        }
        let mut entities: Vec<String> = self
            .entities
            .into_iter()
            .map(|entity| entity.trim().to_lowercase())
            .filter(|entity| !entity.is_empty())
            .collect();
        entities.sort_unstable();
        entities.dedup();
        Some(ExtractedFact { text, entities })
    }
}

/// Canonical form of an entity name: trimmed and lowercased. The one place
/// the rule lives, so a name arriving as a topic, as a relation endpoint, or
/// as an attribute owner always resolves to the SAME entity hub.
fn canonical_entity(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The strict JSON contract the *graph* extraction prompt asks for.
#[derive(serde::Deserialize)]
struct RawExtraction {
    #[serde(default)]
    facts: Vec<RawFact>,
    #[serde(default)]
    relations: Vec<RawRelation>,
    #[serde(default)]
    attributes: Vec<RawAttribute>,
}

#[derive(serde::Deserialize)]
struct RawRelation {
    subject: String,
    predicate: String,
    object: String,
}

#[derive(serde::Deserialize)]
struct RawAttribute {
    entity: String,
    key: String,
    value: serde_json::Value,
}

impl RawExtraction {
    /// Canonicalize and drop the unusable: a relation missing an endpoint or a
    /// label, an attribute missing an owner or a name. A malformed item is
    /// skipped rather than failing the whole passage — one bad triple must not
    /// cost the caller every good fact in the same reply.
    fn into_extraction(self) -> Extraction {
        Extraction {
            facts: self
                .facts
                .into_iter()
                .filter_map(RawFact::into_fact)
                .collect(),
            relations: self
                .relations
                .into_iter()
                .filter_map(RawRelation::into_relation)
                .collect(),
            attributes: self
                .attributes
                .into_iter()
                .filter_map(RawAttribute::into_attribute)
                .collect(),
        }
    }
}

impl RawRelation {
    fn into_relation(self) -> Option<ExtractedRelation> {
        let subject = canonical_entity(&self.subject);
        let object = canonical_entity(&self.object);
        let predicate = self.predicate.trim().to_string();
        // A self-loop carries no information and would sit in the graph as a
        // permanent dead end, so it is dropped alongside the incomplete ones.
        if subject.is_empty() || object.is_empty() || predicate.is_empty() || subject == object {
            return None;
        }
        Some(ExtractedRelation {
            subject,
            predicate,
            object,
        })
    }
}

impl RawAttribute {
    fn into_attribute(self) -> Option<ExtractedAttribute> {
        let entity = canonical_entity(&self.entity);
        let key = self.key.trim().to_string();
        // A null value is the model saying "not stated"; storing it would make
        // an absent attribute look like a known-empty one.
        if entity.is_empty() || key.is_empty() || self.value.is_null() {
            return None;
        }
        Some(ExtractedAttribute {
            entity,
            key,
            value: self.value,
        })
    }
}

/// A short, single-line preview of model output for error messages.
fn truncate(text: &str) -> String {
    const LIMIT: usize = 120;
    let mut out = String::new();
    for word in text.split_whitespace() {
        // Check the budget *before* pushing so we never need a post-hoc
        // `String::truncate`, which would panic if the limit fell mid-UTF-8 char.
        let sep_len = usize::from(!out.is_empty());
        if out.len() + sep_len + word.len() > LIMIT {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// Parse `text` into `T`, first slicing out the outermost JSON array/object.
/// Local models usually honour "return only JSON" but occasionally wrap it in
/// fences or a sentence; slicing the first balanced span tolerates that.
fn json_slice<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice(text)?;
    serde_json::from_str::<T>(slice).ok()
}

/// [`json_slice`] for a reply whose top level is a JSON **object**.
///
/// The array-preferring form cannot be reused: the graph reply is
/// `{"facts": [...], ...}`, whose first `[` belongs to a *nested* field, so
/// preferring arrays slices out the inner facts list and then fails to read it
/// as the whole extraction. That failure is invisible to a stub-backed test —
/// only a real model reply goes through this path.
fn json_slice_object<T: serde::de::DeserializeOwned>(text: &str) -> Option<T> {
    let slice = balanced_slice_preferring(text, b'{')?;
    serde_json::from_str::<T>(slice).ok()
}

/// Return the substring spanning the first balanced `[..]` or `{..}`, honouring
/// string literals and escapes so brackets inside quotes don't miscount.
///
/// Prefers an array: the fact-only reply is a JSON list, and prose before it
/// ("Result {ok}: [...]") may carry a stray `{` that would mis-slice the
/// object span instead of the array.
fn balanced_slice(text: &str) -> Option<&str> {
    balanced_slice_preferring(text, b'[')
}

/// [`balanced_slice`] with the caller choosing which delimiter wins when both
/// appear — the shape the caller actually expects at the top level.
fn balanced_slice_preferring(text: &str, preferred: u8) -> Option<&str> {
    let bytes = text.as_bytes();
    let fallback = if preferred == b'[' { b'{' } else { b'[' };
    let start = bytes
        .iter()
        .position(|&b| b == preferred)
        .or_else(|| bytes.iter().position(|&b| b == fallback))?;
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

#[cfg(test)]
#[path = "reply_tests.rs"]
mod tests;
