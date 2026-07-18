//! Inline media handling for context fragments (US-009, PR1).
//!
//! Two responsibilities, kept separate from the rest of the pipeline so they
//! stay independently testable: a dependency-free base64 decoder (the
//! `context` feature ships zero new dependencies by design — see its doc in
//! `src/context.rs` — so this stays local rather than pulling in the
//! `base64` crate), and the per-fragment analysis ([`analyze`]) that reduces
//! a decoded payload to exactly what the compiler pipeline needs: a
//! raw-bytes dedup identity and a precomputed image token cost.

use crate::id::stable_id_bytes;

use super::estimator::ImageTokenEstimator;
use super::model::MediaRef;

/// Everything the pipeline needs from one fragment's media payload, computed
/// once per fragment and reused by dedup, packing, and insights.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MediaAnalysis {
    /// FNV-1a 64 ([`stable_id_bytes`]) of the *decoded* raw bytes — the
    /// dedup identity for media (byte-identical only; media is never
    /// near-duplicated, see [`super::dedup::find_duplicates`]).
    pub raw_hash: u64,
    /// Precomputed image token cost from [`ImageTokenEstimator`]. This is
    /// the image's cost alone — it does not include the fragment's caption
    /// text; callers fold that in separately (see
    /// `context::media_fragment_tokens`) so the two concerns stay
    /// independently testable.
    pub image_tokens: u64,
}

/// Decode and analyze one fragment's media payload.
///
/// Called only after [`validate_media`] has already confirmed `bytes_b64` is
/// well-formed and within [`crate::limits::MAX_MEDIA_BYTES`] for every
/// fragment in the request (so each payload is decoded twice — once to
/// validate, once here; bounded by
/// [`crate::limits::MAX_TOTAL_MEDIA_BYTES`], a deliberate simplicity
/// trade-off over threading decoded bytes through the pipeline) — a decode failure here would mean that check has
/// a bug, so this degrades to an empty payload (zero dimensions, the text
/// fallback cost) rather than panicking on what is, by this point, trusted
/// input.
pub(crate) fn analyze(media: &MediaRef) -> MediaAnalysis {
    let bytes = decode_base64(&media.bytes_b64).unwrap_or_default();
    MediaAnalysis {
        raw_hash: stable_id_bytes(&bytes),
        image_tokens: ImageTokenEstimator::estimate(&media.mime, &bytes, &media.bytes_b64),
    }
}

/// Whether `bytes_b64` is well-formed base64 — used at validation time to
/// reject a malformed payload before any decode-dependent work happens.
pub(crate) fn is_valid_base64(bytes_b64: &str) -> bool {
    decode_base64(bytes_b64).is_ok()
}

/// Base64 decoding failed: wrong length, an invalid character, or
/// misplaced/excess padding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecodeError;

/// A minimal, dependency-free base64 decoder: standard alphabet
/// (`A-Za-z0-9+/`), `=` padding, no line breaks or whitespace tolerance (the
/// wire payload is a single JSON string, never a formatted block). Not
/// constant-time — this decodes opaque wire-format media, not a secret.
pub(crate) fn decode_base64(input: &str) -> Result<Vec<u8>, DecodeError> {
    let bytes = input.as_bytes();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.len().is_multiple_of(4) {
        return Err(DecodeError);
    }
    let quad_count = bytes.len() / 4;
    let mut out = Vec::with_capacity(quad_count * 3);
    for (index, quad) in bytes.chunks_exact(4).enumerate() {
        let pad = decode_quad(quad, &mut out)?;
        // Padding may only terminate the stream: a quad with padding that
        // is not the last quad means an earlier segment was truncated.
        if pad > 0 && index + 1 != quad_count {
            return Err(DecodeError);
        }
    }
    Ok(out)
}

/// Decode one 4-character quad into up to 3 output bytes, returning the
/// number of trailing `=` padding characters (0, 1, or 2) it carried.
fn decode_quad(quad: &[u8], out: &mut Vec<u8>) -> Result<usize, DecodeError> {
    let pad = quad.iter().rev().take_while(|&&b| b == b'=').count();
    if pad > 2 {
        return Err(DecodeError);
    }
    let mut sextets = [0_u8; 4];
    for (index, &byte) in quad.iter().enumerate() {
        sextets[index] = if byte == b'=' {
            // A '=' is only valid in the trailing `pad` positions; one
            // appearing before that (a data position) is malformed.
            if index < 4 - pad {
                return Err(DecodeError);
            }
            0
        } else {
            sextet(byte).ok_or(DecodeError)?
        };
    }
    let word = (u32::from(sextets[0]) << 18)
        | (u32::from(sextets[1]) << 12)
        | (u32::from(sextets[2]) << 6)
        | u32::from(sextets[3]);
    #[allow(clippy::cast_possible_truncation)]
    out.push((word >> 16) as u8);
    if pad < 2 {
        #[allow(clippy::cast_possible_truncation)]
        out.push((word >> 8) as u8);
    }
    if pad < 1 {
        #[allow(clippy::cast_possible_truncation)]
        out.push(word as u8);
    }
    Ok(pad)
}

/// One base64 character's 6-bit value, or `None` outside the alphabet.
fn sextet(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
#[path = "media_tests.rs"]
mod tests;
