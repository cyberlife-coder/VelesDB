//! [`OutlineExtractor`]: the deterministic, network-free backend.

use super::{
    ExtractError, ExtractedAttribute, ExtractedFact, ExtractedRelation, Extraction, Extractor,
};

// --- Always-available backend: the outline a passage states -------------------
//
// The twin of `HashEmbedder` on this side of the crate, and for the same
// reason: without a dependency-free choice, every contract
// `remember_extracted` publishes is reachable only through a network call, so
// no binding can exercise it and no test can prove it. Deliberately NOT behind
// `extractor-http` — that feature exists to pull in the HTTP client, which this
// backend does not need.

/// Deterministic, network-free extractor: it reads the structure a passage
/// STATES instead of inferring it.
///
/// A generative backend guesses which facts a paragraph holds; this one is
/// told. Each non-blank line of the passage is one directive:
///
/// | line | yields |
/// |---|---|
/// | `edge: <subject> \| <predicate> \| <object>` | one [`ExtractedRelation`] |
/// | `attr: <entity> \| <key> \| <json value>` | one [`ExtractedAttribute`] |
/// | `fact: <text> \| <topic>, <topic>` | one [`ExtractedFact`] |
/// | anything else | one [`ExtractedFact`], no topics |
///
/// Entity names are canonicalized (trimmed, lowercased) exactly as
/// [`ExtractedFact::entities`] are, so they resolve to the SAME
/// content-addressed hubs a generative backend's would — the two backends can
/// write into one graph.
///
/// Its purpose matches [`crate::HashEmbedder`]'s: reproducible tests and
/// offline behavior. It reads no natural language, so a caller holding only
/// prose wants a generative backend. What it offers instead is the one thing a
/// model cannot: the graph is exactly the one the caller wrote down — up to
/// `orient_kinship`, the repointing pass EVERY backend's relations go
/// through, which can flip a triple whose predicate is a kinship noun the
/// passage also states possessively.
///
/// A malformed directive is an [`ExtractError::Parse`], never a silently
/// dropped line: a graph that quietly loses half of what it was handed is
/// worse than one that refuses.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutlineExtractor;

/// The `|`-separated fields of one directive body, trimmed.
fn directive_fields(rest: &str) -> Vec<&str> {
    rest.split('|').map(str::trim).collect()
}

/// The error a directive carrying the wrong number of fields deserves.
fn wrong_field_count(kind: &str, expected: usize, given: usize) -> ExtractError {
    ExtractError::Parse(format!(
        "`{kind}:` takes {expected} `|`-separated fields, {given} given"
    ))
}

/// `edge: <subject> | <predicate> | <object>`.
fn parse_edge(rest: &str) -> Result<ExtractedRelation, ExtractError> {
    let fields = directive_fields(rest);
    let [subject, predicate, object] = fields[..] else {
        return Err(wrong_field_count("edge", 3, fields.len()));
    };
    if subject.is_empty() || predicate.is_empty() || object.is_empty() {
        return Err(ExtractError::Parse(
            "`edge:` takes a non-blank subject, predicate and object".to_owned(),
        ));
    }
    Ok(ExtractedRelation {
        subject: crate::service::canonical_entity_name(subject),
        predicate: predicate.to_owned(),
        object: crate::service::canonical_entity_name(object),
    })
}

/// `attr: <entity> | <key> | <json value>`.
fn parse_attr(rest: &str) -> Result<ExtractedAttribute, ExtractError> {
    let fields = directive_fields(rest);
    let [entity, key, value] = fields[..] else {
        return Err(wrong_field_count("attr", 3, fields.len()));
    };
    if entity.is_empty() || key.is_empty() {
        return Err(ExtractError::Parse(
            "`attr:` takes a non-blank entity and key".to_owned(),
        ));
    }
    // Parsed as JSON, not stored as text, because `recall_where` comparisons
    // are type-strict: an age handed over as `"15"` would never match a
    // numeric filter (see [`ExtractedAttribute::value`]).
    let value = serde_json::from_str(value)
        .map_err(|err| ExtractError::Parse(format!("`attr:` value is not JSON: {err}")))?;
    Ok(ExtractedAttribute {
        entity: crate::service::canonical_entity_name(entity),
        key: key.to_owned(),
        value,
    })
}

/// `fact: <text> | <topic>, <topic>`, and the fallback for any other line.
fn parse_fact(body: &str) -> Result<ExtractedFact, ExtractError> {
    let (text, topics) = body.split_once('|').unwrap_or((body, ""));
    let text = text.trim();
    if text.is_empty() {
        return Err(ExtractError::Parse(
            "a fact line takes a non-blank text".to_owned(),
        ));
    }
    Ok(ExtractedFact {
        text: text.to_owned(),
        entities: topics
            .split(',')
            .map(crate::service::canonical_entity_name)
            .filter(|topic| !topic.is_empty())
            .collect(),
    })
}

impl Extractor for OutlineExtractor {
    fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        Ok(self.extract_graph(text)?.facts)
    }

    fn extract_graph(&self, text: &str) -> Result<Extraction, ExtractError> {
        let mut extraction = Extraction::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("edge:") {
                extraction.relations.push(parse_edge(rest)?);
            } else if let Some(rest) = line.strip_prefix("attr:") {
                extraction.attributes.push(parse_attr(rest)?);
            } else {
                extraction
                    .facts
                    .push(parse_fact(line.strip_prefix("fact:").unwrap_or(line))?);
            }
        }
        Ok(extraction)
    }
}
