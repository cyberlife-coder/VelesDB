//! Deterministic transcript segmentation for the `compile_transcript` MCP
//! tool (V2b-2, see the crate's `PLAN.md`, section V2b).
//!
//! [`segment_transcript`] turns a raw agent-session transcript — plain text
//! with role markers, or JSONL — into an ordered list of
//! [`TranscriptSegment`]s, each wrapping an ordinary [`super::ContextFragment`]
//! plus the audit metadata (`turn`, `role`, `kind`, byte range) the
//! `compile_transcript` tool reports alongside the compiled context. The
//! resulting fragments feed the existing, unmodified [`super::ContextCompiler`]
//! pipeline — this module only decides *how to cut the transcript up*, never
//! what to keep or drop.
//!
//! **Zero regex, zero clock, single linear scan per stage** — same
//! determinism contract as [`super::chunk`]: the same transcript + the same
//! [`SegmentationPolicy`] always segment byte-identically (see
//! `segmentation_twice_is_byte_identical` in the test suite).
//!
//! # Pipeline
//!
//! 1. **Format detection** ([`detect_and_segment`]): `jsonl` when every
//!    non-empty line parses as a `{role, content}` JSON object, `plain`
//!    otherwise. A caller-forced format that does not parse is a hard error —
//!    never a silent fallback to the other format.
//! 2. **Turns**: `jsonl` — one line, one turn, `role` taken directly from the
//!    parsed JSON. `plain` — a CLOSED table of markers (`"System:"`,
//!    `"User:"`, `"Human:"`, `"Assistant:"`, `"AI:"`, `"Tool:"`,
//!    `"### User"`, `"### Assistant"`), first match at the start of a line
//!    opens a new turn; a transcript with no marker at all is one turn with
//!    `role: None`.
//! 3. **Sub-segmentation** (`plain` turns only — a `jsonl` turn's `content` is
//!    a JSON-decoded string, not a byte-aligned slice of the transcript, so
//!    it is never re-scanned; the underlying `content.contains("```")` /
//!    value-density rules in [`super::classify`] still see it, unaffected):
//!    fenced code blocks ([`super::chunk::fence_segments`]) become atomic
//!    `code` segments; runs of at least 8 consecutive log-like lines (a
//!    volatile timestamp/pid prefix — [`super::log_normalize::mask_volatile_prefix`]
//!    — or a raw-text repeat) become `log` segments; everything else is
//!    `body`.
//! 4. **Normalization**: an unsplittable fence over
//!    [`crate::limits::MAX_FRAGMENT_BYTES`] is a hard error (never silently
//!    truncated); an oversized `body` segment is re-split with
//!    [`super::chunk_text`]; segments under
//!    [`SegmentationPolicy::min_segment_bytes`] merge into an adjacent
//!    segment of the *same turn and kind*; more than
//!    [`crate::limits::MAX_FRAGMENTS`] segments after merging is a hard,
//!    actionable error ("raise `min_segment_bytes`") — never a silent drop.
//!
//! Every error surfaces as [`crate::error::MemoryError::ContextOverLimit`] or
//! [`crate::error::MemoryError::IngestDisabled`]/[`crate::error::MemoryError::IngestOutsideRoots`]/
//! [`crate::error::MemoryError::IngestPath`] (the last three only for a
//! `path`-sourced transcript, via [`super::ingest::resolve_transcript_path`])
//! — the same `INVALID_PARAMS`-category taxonomy `compile_context` already
//! uses, deliberately not a new variant for this PR.

use std::collections::BTreeMap;
use std::ops::Range;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::chunk::{self, chunk_text, ChunkBoundary, ChunkPolicy};
use super::log_normalize::mask_volatile_prefix;
use super::model::ContextFragment;
use crate::error::MemoryError;
use crate::limits::{MAX_FRAGMENTS, MAX_FRAGMENT_BYTES, MAX_TRANSCRIPT_BYTES};

/// A contiguous run of at least this many candidate log lines becomes a
/// `log` segment (see the module docs' step 3). Chosen high enough that an
/// ordinary short warning burst stays `body` (nothing to abstract), low
/// enough that a real log dump — which `abstract.log_dedup` exists to
/// collapse — is reliably recognized.
const MIN_LOG_RUN_LINES: usize = 8;

/// Which transcript format to assume, or detect automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SegmentFormat {
    /// Detect `jsonl` vs `plain` from the transcript itself (the default).
    Auto,
    /// Force plain-text, marker-based turn splitting — a transcript that
    /// happens to also be valid JSONL is still segmented as plain text.
    Plain,
    /// Force one-line-one-turn JSONL parsing — a line that does not parse as
    /// a `{role, content}` object is a hard error, never a silent fallback.
    Jsonl,
}

/// What kind of content a sub-segment carries — decides whether it was cut
/// out as an atomic fence, a detected log run, or ordinary prose/dialogue
/// left for [`super::classify`]'s rule table to judge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SegmentKind {
    /// Ordinary text — [`ContextFragment::kind`] stays `None`, so the
    /// existing classification rules (code fence, URL, negative constraint,
    /// value density, …) decide its fate exactly as for `compile_context`.
    Body,
    /// A triple-backtick-fenced block, cut out atomically by
    /// [`super::chunk::fence_segments`]. Tagged `kind = "code"` so
    /// [`super::classify::classify`]'s `preserve.code_fence` rule matches
    /// even for a fence whose content does not itself literally contain
    /// `` ``` `` (defense in depth; it usually does).
    Code,
    /// A run of at least [`MIN_LOG_RUN_LINES`] log-like lines. Tagged
    /// `kind = "log"` so `abstract.log_dedup` can consider it for
    /// repeated-line collapsing exactly like a caller-declared `kind: "log"`
    /// fragment in `compile_context`.
    Log,
}

impl SegmentKind {
    /// The [`ContextFragment::kind`] hint this segment kind maps to —
    /// `None` for `body` (let the rule table decide unconstrained).
    fn fragment_kind(self) -> Option<&'static str> {
        match self {
            Self::Body => None,
            Self::Code => Some("code"),
            Self::Log => Some("log"),
        }
    }
}

/// Tuning knobs for [`segment_transcript`]. `Default` is the recommended
/// profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct SegmentationPolicy {
    /// Which format to assume (see [`SegmentFormat`]). Default [`SegmentFormat::Auto`].
    pub format: SegmentFormat,
    /// Segments under this many bytes merge into an adjacent segment of the
    /// same turn and kind (see the module docs' step 4). Default `256`.
    pub min_segment_bytes: usize,
    /// When `true` (the default) and [`SegmentationPolicy::format`]
    /// determines the FIRST turn's role is `"system"` (case-insensitive),
    /// every segment of that turn is marked `metadata.cache = true` — the
    /// same signal `compile_context`'s `cache.stable_prefix` rule reads, so
    /// a system prompt turn becomes the compiled output's stable,
    /// cache-friendly prefix without the caller hand-annotating it.
    pub cache_system_turn: bool,
}

impl Default for SegmentationPolicy {
    fn default() -> Self {
        Self {
            format: SegmentFormat::Auto,
            min_segment_bytes: 256,
            cache_system_turn: true,
        }
    }
}

/// One segmented piece of the transcript: an ordinary [`ContextFragment`]
/// (ready to feed [`super::ContextCompiler`]) plus the audit metadata the
/// `compile_transcript` tool reports in its `segmentation.segments` list.
#[derive(Debug, Clone)]
pub struct TranscriptSegment {
    /// The fragment this segment produces — feed it straight into a
    /// [`super::CompileRequest::fragments`] list.
    pub fragment: ContextFragment,
    /// Which turn (0-based, transcript order) this segment belongs to.
    pub turn: usize,
    /// The turn's role, when one was determined (a marker match in `plain`
    /// mode, or the parsed `role` field in `jsonl` mode). `None` for a
    /// `plain` transcript with no matching marker at all.
    pub role: Option<String>,
    /// What kind of content this segment carries.
    pub kind: SegmentKind,
    /// Start byte offset (inclusive) of this segment in the ORIGINAL
    /// transcript text.
    pub byte_start: usize,
    /// End byte offset (exclusive) of this segment in the ORIGINAL
    /// transcript text.
    pub byte_end: usize,
}

/// The full result of [`segment_transcript`]: the detected format, the
/// segments, and how much normalization merging did.
#[derive(Debug, Clone)]
pub struct SegmentationOutcome {
    /// `jsonl` or `plain` — never [`SegmentFormat::Auto`], which only ever
    /// names a caller's REQUEST, not a detected outcome.
    pub format_detected: SegmentFormat,
    /// The final segments, in transcript order.
    pub segments: Vec<TranscriptSegment>,
    /// How many segments the [`SegmentationPolicy::min_segment_bytes`] merge
    /// step eliminated (`pieces_before_merge - segments.len()`).
    pub merged_segments: usize,
}

/// Segment `text` under `policy` — see the module docs for the full
/// pipeline. Pure: no I/O, no clock, no randomness; the same `text` +
/// `policy` always produce byte-identical output.
///
/// # Errors
/// [`MemoryError::ContextOverLimit`] when `text` exceeds
/// [`MAX_TRANSCRIPT_BYTES`], when [`SegmentFormat::Jsonl`] is forced but a
/// line does not parse as a `{role, content}` object, when an unsplittable
/// fence exceeds [`MAX_FRAGMENT_BYTES`], or when the segment count after
/// merging still exceeds [`MAX_FRAGMENTS`].
pub fn segment_transcript(
    text: &str,
    policy: &SegmentationPolicy,
) -> Result<SegmentationOutcome, MemoryError> {
    if text.len() > MAX_TRANSCRIPT_BYTES {
        return Err(MemoryError::ContextOverLimit(format!(
            "transcript of {} bytes exceeds the cap of {MAX_TRANSCRIPT_BYTES} bytes",
            text.len()
        )));
    }

    let (format_detected, pieces) = detect_and_segment(text, policy.format)?;
    reject_oversized_fences(&pieces)?;
    let pieces = resplit_oversized_bodies(text, pieces);
    let pieces_before_merge = pieces.len();
    let merged = merge_tiny(pieces, policy.min_segment_bytes);
    if merged.len() > MAX_FRAGMENTS {
        return Err(MemoryError::ContextOverLimit(format!(
            "transcript segmented into {} fragments, exceeding the cap of {MAX_FRAGMENTS} — \
             raise segmentation.min_segment_bytes to merge more small segments",
            merged.len()
        )));
    }
    let merged_segments = pieces_before_merge - merged.len();
    let segments = merged
        .into_iter()
        .map(|piece| build_segment(text, piece, policy))
        .collect();
    Ok(SegmentationOutcome {
        format_detected,
        segments,
        merged_segments,
    })
}

// --- Raw (pre-normalization) pieces -----------------------------------------

/// A sub-segment before normalization: still tied to the ORIGINAL text's byte
/// range, except `content_override` — set only for a `jsonl` turn (and its
/// re-split children), whose fragment content is a JSON-decoded string with
/// no byte-aligned slice of the raw transcript (JSON escaping means the
/// decoded text is not a substring of the source bytes). When set, `range`
/// still names the raw JSON line's span (needed so the segmentation-wide
/// byte ranges keep partitioning the transcript), but the fragment's
/// `content` comes from `content_override`, never `text[range]`.
struct RawPiece {
    kind: SegmentKind,
    range: Range<usize>,
    turn: usize,
    role: Option<String>,
    content_override: Option<String>,
}

/// Detect the format and produce the initial (pre-normalization) pieces in
/// one pass — for `jsonl` this avoids parsing every line twice (once to
/// detect, once to build).
fn detect_and_segment(
    text: &str,
    requested: SegmentFormat,
) -> Result<(SegmentFormat, Vec<RawPiece>), MemoryError> {
    match requested {
        SegmentFormat::Plain => Ok((SegmentFormat::Plain, plain_pieces(text))),
        SegmentFormat::Jsonl => {
            let pieces = jsonl_pieces(text).map_err(MemoryError::ContextOverLimit)?;
            Ok((SegmentFormat::Jsonl, pieces))
        }
        SegmentFormat::Auto => {
            if !text.is_empty() {
                if let Ok(pieces) = jsonl_pieces(text) {
                    return Ok((SegmentFormat::Jsonl, pieces));
                }
            }
            Ok((SegmentFormat::Plain, plain_pieces(text)))
        }
    }
}

// --- JSONL -------------------------------------------------------------------

/// One JSONL line's required shape. Both fields are mandatory: a line
/// missing either — or not a JSON object at all — fails to parse, which
/// [`detect_and_segment`] treats as "not jsonl" in [`SegmentFormat::Auto`]
/// and as a hard error under a forced [`SegmentFormat::Jsonl`].
#[derive(Deserialize)]
struct JsonlLine {
    role: String,
    content: String,
}

/// Parse every line of `text` as one JSONL turn. `Err` names the first
/// (1-based) offending line — the first failure short-circuits, so a caller
/// forcing `jsonl` on a bad transcript gets an actionable pointer instead of
/// a generic "not jsonl".
fn jsonl_pieces(text: &str) -> Result<Vec<RawPiece>, String> {
    let mut pieces = Vec::new();
    let mut cursor = 0_usize;
    for (turn, line) in text.split_inclusive('\n').enumerate() {
        let start = cursor;
        cursor += line.len();
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let parsed: JsonlLine = serde_json::from_str(trimmed).map_err(|err| {
            format!(
                "jsonl line {}: not a valid {{role, content}} object: {err}",
                turn + 1
            )
        })?;
        pieces.push(RawPiece {
            kind: SegmentKind::Body,
            range: start..cursor,
            turn,
            role: Some(parsed.role),
            content_override: Some(parsed.content),
        });
    }
    Ok(pieces)
}

// --- Plain ---------------------------------------------------------------

/// The CLOSED table of plain-text turn markers, checked in order — the first
/// one a line starts with wins. Never a caller-supplied pattern, so turn
/// detection stays deterministic and predictable (a "User:" cited in prose
/// is a known, accepted false positive — see the crate README).
const PLAIN_MARKERS: &[&str] = &[
    "System:",
    "User:",
    "Human:",
    "Assistant:",
    "AI:",
    "Tool:",
    "### User",
    "### Assistant",
];

/// The first [`PLAIN_MARKERS`] entry `line` starts with, if any.
fn match_marker(line: &str) -> Option<&'static str> {
    PLAIN_MARKERS
        .iter()
        .find(|marker| line.starts_with(*marker))
        .copied()
}

/// A marker's role label: `"### User"` → `"User"`, `"System:"` → `"System"`.
fn marker_role(marker: &str) -> String {
    marker
        .strip_prefix("### ")
        .unwrap_or(marker)
        .trim_end_matches(':')
        .to_owned()
}

/// Split `text` into plain-format turns: `(byte_range, role)`, in order,
/// partitioning `text` exactly. No marker anywhere in `text` yields exactly
/// one turn covering the whole text with `role: None`.
fn plain_turns(text: &str) -> Vec<(Range<usize>, Option<String>)> {
    let mut turns = Vec::new();
    let mut turn_start = 0_usize;
    let mut pending_role: Option<String> = None;
    let mut cursor = 0_usize;
    for line in text.split_inclusive('\n') {
        let line_start = cursor;
        if let Some(marker) = match_marker(line) {
            if line_start > turn_start {
                turns.push((turn_start..line_start, pending_role.take()));
            }
            pending_role = Some(marker_role(marker));
            turn_start = line_start;
        }
        cursor += line.len();
    }
    turns.push((turn_start..text.len(), pending_role));
    turns
}

/// Build the initial pieces for a `plain` transcript: turns, then within
/// each turn's slice, fences (atomic `code`) and log runs (`log`), the rest
/// `body` — see the module docs' step 3.
fn plain_pieces(text: &str) -> Vec<RawPiece> {
    let mut pieces = Vec::new();
    for (turn, (range, role)) in plain_turns(text).into_iter().enumerate() {
        if range.is_empty() {
            continue;
        }
        for segment in chunk::fence_segments(&text[range.clone()]) {
            match segment {
                chunk::Segment::Fence(relative) => pieces.push(RawPiece {
                    kind: SegmentKind::Code,
                    range: (range.start + relative.start)..(range.start + relative.end),
                    turn,
                    role: role.clone(),
                    content_override: None,
                }),
                chunk::Segment::Plain(relative) => {
                    let absolute = (range.start + relative.start)..(range.start + relative.end);
                    for (kind, sub_range) in log_split(text, absolute) {
                        pieces.push(RawPiece {
                            kind,
                            range: sub_range,
                            turn,
                            role: role.clone(),
                            content_override: None,
                        });
                    }
                }
            }
        }
    }
    pieces
}

/// Split `range` of `text` into alternating `body`/`log` pieces: a maximal
/// run of at least [`MIN_LOG_RUN_LINES`] consecutive "log-candidate" lines
/// (a volatile timestamp/pid prefix, or a line that repeats elsewhere in
/// `range`) becomes one `log` piece; every other line stays `body`,
/// contiguous runs of it merged into one piece. Single linear scan.
fn log_split(text: &str, range: Range<usize>) -> Vec<(SegmentKind, Range<usize>)> {
    if range.is_empty() {
        return Vec::new();
    }
    let slice = &text[range.clone()];
    let mut lines: Vec<(Range<usize>, &str)> = Vec::new();
    let mut cursor = range.start;
    for line in slice.split_inclusive('\n') {
        let end = cursor + line.len();
        lines.push((cursor..end, line));
        cursor = end;
    }
    if lines.is_empty() {
        return Vec::new();
    }

    let trimmed: Vec<&str> = lines
        .iter()
        .map(|(_, line)| line.trim_end_matches(['\r', '\n']))
        .collect();
    let mut repeat_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for line in &trimmed {
        *repeat_counts.entry(line).or_insert(0) += 1;
    }
    let candidate: Vec<bool> = trimmed
        .iter()
        .map(|line| {
            !line.is_empty() && (mask_volatile_prefix(line).is_some() || repeat_counts[line] > 1)
        })
        .collect();

    let mut pieces = Vec::new();
    let mut body_start: Option<usize> = None;
    let mut index = 0_usize;
    while index < lines.len() {
        if candidate[index] {
            let run_start = index;
            while index < lines.len() && candidate[index] {
                index += 1;
            }
            if index - run_start >= MIN_LOG_RUN_LINES {
                if let Some(start) = body_start.take() {
                    pieces.push((
                        SegmentKind::Body,
                        lines[start].0.start..lines[run_start - 1].0.end,
                    ));
                }
                pieces.push((
                    SegmentKind::Log,
                    lines[run_start].0.start..lines[index - 1].0.end,
                ));
            } else if body_start.is_none() {
                body_start = Some(run_start);
            }
        } else {
            if body_start.is_none() {
                body_start = Some(index);
            }
            index += 1;
        }
    }
    if let Some(start) = body_start {
        pieces.push((
            SegmentKind::Body,
            lines[start].0.start..lines[lines.len() - 1].0.end,
        ));
    }
    pieces
}

// --- Normalization -----------------------------------------------------------

/// Reject an unsplittable fence over [`MAX_FRAGMENT_BYTES`] — a fence is
/// always atomic (never cut, see [`super::chunk`]), so an oversized one
/// cannot be brought under the cap the way a `body` piece can.
///
/// # Errors
/// [`MemoryError::ContextOverLimit`] naming the first oversized fence found.
fn reject_oversized_fences(pieces: &[RawPiece]) -> Result<(), MemoryError> {
    if let Some(piece) = pieces
        .iter()
        .find(|piece| piece.kind == SegmentKind::Code && piece.range.len() > MAX_FRAGMENT_BYTES)
    {
        return Err(MemoryError::ContextOverLimit(format!(
            "an unsplittable fenced code block of {} bytes exceeds the cap of {MAX_FRAGMENT_BYTES} bytes",
            piece.range.len()
        )));
    }
    Ok(())
}

/// Re-split every `body` piece over [`MAX_FRAGMENT_BYTES`] with
/// [`chunk_text`] — the same re-chunker `compile_context` itself uses for an
/// oversized fragment. A `jsonl` piece's decoded `content_override` has no
/// byte-aligned mapping back to the raw (JSON-escaped) source line, so its
/// re-split children all keep the ORIGINAL line's full byte range — a
/// documented, deliberately narrow trade-off: the byte-range-covers-the-
/// transcript property holds at the turn level regardless, and a single
/// JSONL line's `content` exceeding 1 MiB is an extreme edge case.
fn resplit_oversized_bodies(text: &str, pieces: Vec<RawPiece>) -> Vec<RawPiece> {
    let chunk_policy = ChunkPolicy {
        max_chunk_bytes: MAX_FRAGMENT_BYTES,
        overlap_bytes: 0,
        boundary: ChunkBoundary::Paragraph,
    };
    pieces
        .into_iter()
        .flat_map(|piece| resplit_one(text, piece, &chunk_policy))
        .collect()
}

fn resplit_one(text: &str, piece: RawPiece, chunk_policy: &ChunkPolicy) -> Vec<RawPiece> {
    if piece.kind != SegmentKind::Body {
        return vec![piece];
    }
    let effective_len = piece
        .content_override
        .as_ref()
        .map_or(piece.range.len(), String::len);
    if effective_len <= MAX_FRAGMENT_BYTES {
        return vec![piece];
    }
    match &piece.content_override {
        Some(content) => chunk_text(content, chunk_policy)
            .into_iter()
            .map(|chunk| RawPiece {
                kind: SegmentKind::Body,
                range: piece.range.clone(),
                turn: piece.turn,
                role: piece.role.clone(),
                content_override: Some(chunk.text),
            })
            .collect(),
        None => chunk_text(&text[piece.range.clone()], chunk_policy)
            .into_iter()
            .map(|chunk| RawPiece {
                kind: SegmentKind::Body,
                range: (piece.range.start + chunk.byte_range.start)
                    ..(piece.range.start + chunk.byte_range.end),
                turn: piece.turn,
                role: piece.role.clone(),
                content_override: None,
            })
            .collect(),
    }
}

/// Merge adjacent pieces of the SAME turn and kind when either side is under
/// `min_bytes` — see the module docs' step 4. A `jsonl` piece never merges
/// with another (each holds its own unique `turn`, since `jsonl` is
/// one-line-one-turn by construction), nor does any piece carrying a
/// `content_override` (merging would require re-deriving a combined decoded
/// string, which is not meaningful once JSON escaping is involved).
fn merge_tiny(pieces: Vec<RawPiece>, min_bytes: usize) -> Vec<RawPiece> {
    let mut merged: Vec<RawPiece> = Vec::new();
    for piece in pieces {
        let mergeable = merged.last().is_some_and(|last: &RawPiece| {
            last.turn == piece.turn
                && last.kind == piece.kind
                && last.content_override.is_none()
                && piece.content_override.is_none()
                && last.range.end == piece.range.start
                && (last.range.len() < min_bytes || piece.range.len() < min_bytes)
        });
        if mergeable {
            // Safe: `mergeable` only true when `merged` is non-empty.
            merged
                .last_mut()
                .expect("checked non-empty above")
                .range
                .end = piece.range.end;
        } else {
            merged.push(piece);
        }
    }
    merged
}

// --- Assembly ------------------------------------------------------------

/// Build the final [`TranscriptSegment`] for one normalized piece:
/// `metadata = {role, turn}`, plus `cache: true` when
/// [`SegmentationPolicy::cache_system_turn`] applies (turn 0, role
/// case-insensitively `"system"`).
fn build_segment(text: &str, piece: RawPiece, policy: &SegmentationPolicy) -> TranscriptSegment {
    let content = piece
        .content_override
        .clone()
        .unwrap_or_else(|| text[piece.range.clone()].to_owned());

    let mut metadata = Map::new();
    metadata.insert(
        "role".to_owned(),
        piece.role.clone().map_or(Value::Null, Value::String),
    );
    metadata.insert("turn".to_owned(), Value::Number(piece.turn.into()));
    let is_first_turn_system = piece.turn == 0
        && piece
            .role
            .as_deref()
            .is_some_and(|role| role.eq_ignore_ascii_case("system"));
    if policy.cache_system_turn && is_first_turn_system {
        metadata.insert("cache".to_owned(), Value::Bool(true));
    }

    let fragment = ContextFragment {
        id: None,
        content,
        path: None,
        kind: piece.kind.fragment_kind().map(str::to_owned),
        priority: None,
        metadata: Some(metadata),
        media: None,
    };
    TranscriptSegment {
        fragment,
        turn: piece.turn,
        role: piece.role,
        kind: piece.kind,
        byte_start: piece.range.start,
        byte_end: piece.range.end,
    }
}

#[cfg(test)]
#[path = "segment_tests.rs"]
mod tests;
