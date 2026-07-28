//! The one place a *binding* turns a raw transcript into a
//! [`CompileRequest`] plus an auditable segmentation report.
//!
//! `compile_transcript` is a one-call shortcut over `compile_context`: it
//! segments a raw agent-session transcript into turns (and, within a turn,
//! into code/log/body sub-segments) before compiling. The segmentation
//! itself lives in [`super::segment`]; what lives here is the *glue* around
//! it — the empty-transcript guard the MCP tool applies, the request
//! assembly, and the per-segment audit trail a caller inspects to see how
//! its transcript was cut before trusting the compiled result.
//!
//! That glue was copied verbatim into `velesdb-node` and `velesdb-wasm`,
//! each doc comment pointing at the other as "mirrors the … binding's own"
//! — two copies kept in step by hand. The Python binding could not be given
//! `compile_transcript` without becoming a third. It lives here instead, so
//! the three bindings relay one implementation and a fix reaches all of them
//! at once.
//!
//! `fragment_id` is rendered as a decimal string, not a `u64`: it is a
//! 64-bit content hash, routinely past 2^53, and a JS caller reading it as a
//! `number` would round it. Every binding publishes it that way already, and
//! so does the MCP tool — the string is the wire form, not a JS concession.
//!
//! No `path` field: resolving one needs an ingest-roots allowlist, which is
//! MCP-server configuration. A binding caller reads the file itself and
//! passes the text.

use serde::{Deserialize, Serialize};

use super::segment::{segment_transcript, SegmentFormat, SegmentKind, SegmentationPolicy};
use super::{fragment_id, CompilePolicy, CompileRequest};
use crate::error::MemoryError;

/// A binding's `compile_transcript` request: the MCP tool's own fields minus
/// `path` (see the module docs).
#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptCompileInput {
    /// What the compiled context has to serve — drives relevance ranking.
    pub query: String,
    /// The raw transcript, plain marker-based or JSONL.
    pub transcript: String,
    /// Token ceiling for the compiled context.
    pub token_budget: u64,
    /// Optional project key, recorded with the savings event.
    #[serde(default)]
    pub project: Option<String>,
    /// Optional model name the context targets.
    #[serde(default)]
    pub target_model: Option<String>,
    /// Optional compilation policy overrides.
    #[serde(default)]
    pub policy: Option<CompilePolicy>,
    /// Optional segmentation policy overrides.
    #[serde(default)]
    pub segmentation: Option<SegmentationPolicy>,
}

/// One entry of [`SegmentationReport::segments`]: where a fragment came from
/// in the original transcript.
#[derive(Debug, Clone, Serialize)]
pub struct SegmentInfo {
    /// Position of this segment in the compiled fragment list.
    pub index: usize,
    /// 0-based turn the segment was cut from.
    pub turn: usize,
    /// Speaker of that turn, when the transcript labelled one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Whether the segment is body text, fenced code, or a log run.
    pub kind: SegmentKind,
    /// Start offset in the original transcript, in bytes.
    pub byte_start: usize,
    /// End offset in the original transcript, in bytes.
    pub byte_end: usize,
    /// Content-addressed fragment id, as a decimal string (see module docs).
    pub fragment_id: String,
}

/// How a transcript was cut, returned alongside the compiled context so a
/// caller can audit the cut before trusting the result.
#[derive(Debug, Clone, Serialize)]
pub struct SegmentationReport {
    /// Format the segmenter detected (or was forced to).
    pub format_detected: SegmentFormat,
    /// One entry per resulting fragment, in compile order.
    pub segments: Vec<SegmentInfo>,
    /// How many adjacent segments were merged away.
    pub merged_segments: usize,
}

/// Segment `input.transcript` and assemble the [`CompileRequest`] a binding
/// then hands to `compile_context`, plus the [`SegmentationReport`] it
/// returns next to the compiled context.
///
/// # Errors
/// [`MemoryError::SegmentationError`] for an empty transcript — mirroring
/// the MCP tool's own guard, since [`segment_transcript`] has none of its
/// own (an empty string is a valid, if useless, zero-turn input to it) — or
/// whatever [`segment_transcript`] itself returns: a genuine budget/cap
/// breach, or a forced-format parse failure.
pub fn build_transcript_compile_request(
    input: TranscriptCompileInput,
) -> Result<(CompileRequest, SegmentationReport), MemoryError> {
    if input.transcript.is_empty() {
        return Err(MemoryError::SegmentationError(
            "the transcript is empty — `transcript` must be non-empty text".to_owned(),
        ));
    }
    let outcome = segment_transcript(&input.transcript, &input.segmentation.unwrap_or_default())?;
    let segments = outcome
        .segments
        .iter()
        .enumerate()
        .map(|(index, segment)| SegmentInfo {
            index,
            turn: segment.turn,
            role: segment.role.clone(),
            kind: segment.kind,
            byte_start: segment.byte_start,
            byte_end: segment.byte_end,
            fragment_id: fragment_id(&segment.fragment.content).to_string(),
        })
        .collect();
    let report = SegmentationReport {
        format_detected: outcome.format_detected,
        segments,
        merged_segments: outcome.merged_segments,
    };
    let request = CompileRequest {
        query: input.query,
        fragments: outcome.segments.into_iter().map(|s| s.fragment).collect(),
        project: input.project,
        target_model: input.target_model,
        token_budget: input.token_budget,
        memory_scope: None,
        policy: input.policy,
    };
    Ok((request, report))
}

#[cfg(test)]
#[path = "transcript_bridge_tests.rs"]
mod tests;
