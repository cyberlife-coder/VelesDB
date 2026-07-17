//! Deterministic text chunking for the packing stage.
//!
//! No chunker exists anywhere else in the workspace (every other "chunk" is
//! vector batching), so this is the reference implementation. Guarantees:
//!
//! - **Deterministic**: same text + same policy ⇒ same chunks, same ranges.
//! - **UTF-8 safe**: never cuts inside a multi-byte char.
//! - **Fence-atomic**: never cuts inside a triple-backtick-fenced code block — a fence
//!   larger than [`ChunkPolicy::max_chunk_bytes`] stays one oversized chunk
//!   rather than being broken (the packing layer decides its fate whole).
//! - **Covering**: without overlap, the chunk ranges partition the input —
//!   concatenating them reconstructs the text byte for byte.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Which boundaries the chunker prefers to cut at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChunkBoundary {
    /// Cut at blank lines (`\n\n`), falling back to hard splits inside an
    /// oversized paragraph.
    Paragraph,
    /// Cut after sentence enders (`.`, `!`, `?` followed by whitespace).
    Sentence,
    /// Cut at the byte ceiling only (char-aligned).
    Fixed,
}

/// How oversized fragments are split before packing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[schemars(transform = crate::schema::strip_int_formats)]
pub struct ChunkPolicy {
    /// Soft byte ceiling per chunk (fences may exceed it, see module doc).
    pub max_chunk_bytes: usize,
    /// Bytes of the previous chunk to repeat at the start of the next one
    /// (char-aligned). `0` keeps chunks disjoint.
    pub overlap_bytes: usize,
    /// Preferred cut points.
    pub boundary: ChunkBoundary,
}

impl Default for ChunkPolicy {
    fn default() -> Self {
        Self {
            max_chunk_bytes: 2_048,
            overlap_bytes: 0,
            boundary: ChunkBoundary::Paragraph,
        }
    }
}

/// One chunk of a larger text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    /// The chunk text (including any overlap prefix).
    pub text: String,
    /// Where the *non-overlap* part of this chunk sits in the original text.
    pub byte_range: core::ops::Range<usize>,
    /// Position of this chunk in the sequence, starting at `0`.
    pub index: usize,
}

/// Split `text` into chunks under `policy`. Empty text yields no chunks.
#[must_use]
pub fn chunk_text(text: &str, policy: &ChunkPolicy) -> Vec<TextChunk> {
    if text.is_empty() {
        return Vec::new();
    }
    let max = policy.max_chunk_bytes.max(1);
    if text.len() <= max {
        return vec![TextChunk {
            text: text.to_owned(),
            byte_range: 0..text.len(),
            index: 0,
        }];
    }
    let ranges = pack_units(text, &units(text, policy.boundary), max);
    assemble(text, &ranges, policy.overlap_bytes)
}

/// One partition of the text: a byte range plus whether it is atomic
/// (a genuine fenced block — never split, however large) or plain-text-
/// derived (always eligible for a hard split when oversized). `atomic` is
/// carried explicitly rather than re-derived from the range's content, so a
/// plain-text cut that merely happens to start with `` ``` `` (e.g. a
/// sentence boundary landing right before literal backticks in prose) is
/// never mistaken for a real fence.
struct Unit {
    range: core::ops::Range<usize>,
    atomic: bool,
}

/// Split `text` into atomic unit ranges: fenced blocks stay whole, the rest
/// is cut at the preferred boundary. The ranges partition the text.
fn units(text: &str, boundary: ChunkBoundary) -> Vec<Unit> {
    let mut units = Vec::new();
    for segment in fence_segments(text) {
        match segment {
            Segment::Fence(range) => units.push(Unit {
                range,
                atomic: true,
            }),
            Segment::Plain(range) => split_plain(text, range, boundary, &mut units),
        }
    }
    units
}

/// A top-level slice of the text: either a whole fenced block or plain text.
enum Segment {
    /// A triple-backtick-fenced block, atomic.
    Fence(core::ops::Range<usize>),
    /// Plain text between fences.
    Plain(core::ops::Range<usize>),
}

/// Walk the text line by line, separating triple-backtick-fenced blocks (atomic) from the
/// plain text around them. An unclosed fence runs to the end of the text.
fn fence_segments(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut cursor = 0_usize;
    let mut fence_start: Option<usize> = None;
    let mut line_start = 0_usize;
    for line in text.split_inclusive('\n') {
        let opens_or_closes = line.trim_start().starts_with("```");
        let line_end = line_start + line.len();
        match (fence_start, opens_or_closes) {
            (None, true) => {
                if line_start > cursor {
                    segments.push(Segment::Plain(cursor..line_start));
                }
                fence_start = Some(line_start);
            }
            (Some(start), true) => {
                segments.push(Segment::Fence(start..line_end));
                fence_start = None;
                cursor = line_end;
            }
            _ => {}
        }
        line_start = line_end;
    }
    push_tail(&mut segments, fence_start, cursor, text.len());
    segments
}

/// Close the segment walk: an unclosed fence (or the trailing plain text)
/// runs to the end of the input.
fn push_tail(segments: &mut Vec<Segment>, fence_start: Option<usize>, cursor: usize, end: usize) {
    match fence_start {
        Some(start) => segments.push(Segment::Fence(start..end)),
        None if cursor < end => segments.push(Segment::Plain(cursor..end)),
        None => {}
    }
}

/// Cut one plain-text range at the preferred boundary, appending the pieces
/// — always non-atomic: nothing derived from a `Plain` segment is ever a
/// real fence, however it happens to start.
fn split_plain(
    text: &str,
    range: core::ops::Range<usize>,
    boundary: ChunkBoundary,
    out: &mut Vec<Unit>,
) {
    let slice = &text[range.clone()];
    let mut piece_start = 0_usize;
    for cut in boundary_cuts(slice, boundary) {
        out.push(Unit {
            range: range.start + piece_start..range.start + cut,
            atomic: false,
        });
        piece_start = cut;
    }
    if piece_start < slice.len() {
        out.push(Unit {
            range: range.start + piece_start..range.end,
            atomic: false,
        });
    }
}

/// The byte offsets (relative to `slice`) *after* which a boundary cut is
/// allowed. Offsets are strictly increasing and land on char boundaries.
fn boundary_cuts(slice: &str, boundary: ChunkBoundary) -> Vec<usize> {
    match boundary {
        ChunkBoundary::Paragraph => paragraph_cuts(slice),
        ChunkBoundary::Sentence => sentence_cuts(slice),
        ChunkBoundary::Fixed => Vec::new(),
    }
}

/// Cut points after each blank-line run (`\n\n…`).
fn paragraph_cuts(slice: &str) -> Vec<usize> {
    let mut cuts = Vec::new();
    let bytes = slice.as_bytes();
    let mut i = 0_usize;
    while let Some(found) = find_from(bytes, i, b"\n\n") {
        let mut end = found + 2;
        while bytes.get(end) == Some(&b'\n') {
            end += 1;
        }
        cuts.push(end);
        i = end;
    }
    cuts
}

/// Cut points after each sentence ender followed by whitespace.
fn sentence_cuts(slice: &str) -> Vec<usize> {
    let mut cuts = Vec::new();
    let mut previous: Option<char> = None;
    for (offset, ch) in slice.char_indices() {
        let after_ender = matches!(previous, Some('.' | '!' | '?'));
        if after_ender && ch.is_whitespace() {
            cuts.push(offset + ch.len_utf8());
        }
        previous = Some(ch);
    }
    cuts
}

/// Find `needle` in `haystack` at or after `from`.
fn find_from(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|position| from + position)
}

/// Greedily merge units into chunk ranges of at most `max` bytes. A single
/// unit larger than `max` is hard-split at char boundaries — unless it is
/// marked atomic (a genuine fence), which stays whole regardless of size.
fn pack_units(text: &str, units: &[Unit], max: usize) -> Vec<core::ops::Range<usize>> {
    let mut chunks: Vec<core::ops::Range<usize>> = Vec::new();
    let mut open: Option<core::ops::Range<usize>> = None;
    for unit in units {
        if unit.range.len() > max {
            flush(&mut chunks, &mut open);
            append_oversized(text, unit, max, &mut chunks);
        } else {
            open = Some(merge_or_flush(&mut chunks, open, unit.range.clone(), max));
        }
    }
    flush(&mut chunks, &mut open);
    chunks
}

/// Extend the open chunk with `unit` if it fits, otherwise seal it and open a
/// new chunk at `unit`; returns the chunk left open.
fn merge_or_flush(
    chunks: &mut Vec<core::ops::Range<usize>>,
    open: Option<core::ops::Range<usize>>,
    unit: core::ops::Range<usize>,
    max: usize,
) -> core::ops::Range<usize> {
    match open {
        Some(range) if unit.end - range.start <= max => range.start..unit.end,
        Some(range) => {
            chunks.push(range);
            unit
        }
        None => unit,
    }
}

/// Seal the open chunk, if any.
fn flush(chunks: &mut Vec<core::ops::Range<usize>>, open: &mut Option<core::ops::Range<usize>>) {
    if let Some(range) = open.take() {
        chunks.push(range);
    }
}

/// Append an oversized unit: an atomic unit (a genuine fence) stays whole,
/// everything else hard-splits at char boundaries every `max` bytes — the
/// unit's `atomic` flag decides, never its leading bytes (see [`Unit`]).
fn append_oversized(
    text: &str,
    unit: &Unit,
    max: usize,
    chunks: &mut Vec<core::ops::Range<usize>>,
) {
    if unit.atomic {
        chunks.push(unit.range.clone());
        return;
    }
    let mut start = unit.range.start;
    while start < unit.range.end {
        let floored = char_floor(text, (start + max).min(unit.range.end));
        // A ceiling smaller than the char at `start` cannot cut inside it:
        // advance to the next char boundary instead of forcing a mid-char cut.
        let end = if floored > start {
            floored
        } else {
            char_ceil(text, start + 1).min(unit.range.end)
        };
        chunks.push(start..end);
        start = end;
    }
}

/// The largest char boundary at or below `at`.
fn char_floor(text: &str, at: usize) -> usize {
    let mut boundary = at.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

/// Materialize chunk ranges into [`TextChunk`]s, prepending the char-aligned
/// overlap tail of the previous chunk when the policy asks for one.
fn assemble(
    text: &str,
    ranges: &[core::ops::Range<usize>],
    overlap_bytes: usize,
) -> Vec<TextChunk> {
    ranges
        .iter()
        .enumerate()
        .map(|(index, range)| {
            let mut chunk_text = String::new();
            if overlap_bytes > 0 && index > 0 {
                let overlap_start = char_ceil(text, range.start.saturating_sub(overlap_bytes));
                chunk_text.push_str(&text[overlap_start..range.start]);
            }
            chunk_text.push_str(&text[range.clone()]);
            TextChunk {
                text: chunk_text,
                byte_range: range.clone(),
                index,
            }
        })
        .collect()
}

/// The smallest char boundary at or above `at`.
fn char_ceil(text: &str, at: usize) -> usize {
    let mut boundary = at.min(text.len());
    while !text.is_char_boundary(boundary) {
        boundary += 1;
    }
    boundary
}

#[cfg(test)]
#[path = "chunk_tests.rs"]
mod tests;
