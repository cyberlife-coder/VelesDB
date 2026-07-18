//! Pluggable token estimation, with a deterministic char-class default.
//!
//! Same shape as [`crate::embedder`]: a small trait, one dependency-free
//! default, and a boxed alias for non-generic holders. Estimates here are
//! **local approximations** — distinct from a provider's exact tokenizer
//! count, from billed tokens, and from cache-read tokens. Budget packing
//! treats them as an over-approximation on purpose: refusing a borderline
//! fragment is recoverable (it becomes a retrieval handle), overflowing the
//! window is not.

/// Turns text into an estimated token count.
pub trait TokenEstimator {
    /// Estimated number of tokens `text` occupies in a model prompt.
    fn estimate(&self, text: &str) -> u64;

    /// Rough bytes-per-token ratio of this estimator, used only as a *hint*
    /// to size chunk pieces near the budget (every piece is still measured
    /// by [`Self::estimate`] during packing, so a wrong hint costs
    /// granularity, never correctness). The default matches the char-class
    /// estimator's prose rate; a model-exact tokenizer for dense scripts
    /// (CJK) should lower it.
    fn bytes_per_token_hint(&self) -> u64 {
        3
    }
}

/// A boxed, object-safe estimator, mirroring [`crate::embedder::DynEmbedder`].
pub type DynTokenEstimator = Box<dyn TokenEstimator + Send + Sync>;

/// Forward [`TokenEstimator`] through a box so a non-generic compiler can
/// hold [`DynTokenEstimator`].
impl<T: TokenEstimator + ?Sized> TokenEstimator for Box<T> {
    fn estimate(&self, text: &str) -> u64 {
        (**self).estimate(text)
    }

    fn bytes_per_token_hint(&self) -> u64 {
        (**self).bytes_per_token_hint()
    }
}

/// Deterministic char-class estimator, calibrated against a real BPE
/// (cl100k) on a mixed corpus. Per whitespace-separated word, each char
/// costs: CJK **5/6** token, ASCII digit **1** token, anything else
/// **3/10** token; the word's cost is the ceiling of the sum. Inter-word
/// spaces and tabs are free (BPE folds them into the following token), but
/// each **newline** costs half a token — cl100k spends ~one token per
/// newline run — added on top of the per-word sum (see `estimate`).
///
/// Measured margins vs cl100k (estimate − real, positive = safe over-count):
/// English prose **+55 %**, French prose **+38 %**, repetitive logs
/// **+52 %**, Rust code **+19 %**, URLs **+20 %**, Markdown **+16 %**, JSON
/// **+13 %**, digit-dense ids/dates **+29 %**, CJK **+14 %**. The per-word
/// ceiling keeps the estimate superadditive (summing piece estimates bounds
/// the estimate of their concatenation), which is what makes the packing
/// budget guarantee hold.
///
/// Known adversarial bias: words made purely of hex *letters*
/// (`deadbeef cafebabe …`) tokenize like digits but cost like prose, and a
/// corpus made of them measures ~18 % *under*. For id-dense corpora against
/// a tight budget, inject a model-exact [`TokenEstimator`] instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicEstimator;

/// Per-char costs in thirtieths of a token (common denominator of the
/// calibrated 5/6, 1, and 3/10 rates).
const CJK_THIRTIETHS: u64 = 25;
const DIGIT_THIRTIETHS: u64 = 30;
const OTHER_THIRTIETHS: u64 = 9;

impl TokenEstimator for HeuristicEstimator {
    fn estimate(&self, text: &str) -> u64 {
        let words = text
            .split_whitespace()
            .map(word_cost)
            .fold(0, u64::saturating_add);
        // Spaces and tabs are free (BPE folds them into the next token), but
        // newlines are not: cl100k spends ~one token per newline run, so each
        // '\n' costs half a token (a lone '\n' rounds up to 1, "\n\n" is 1).
        let newlines =
            u64::try_from(text.bytes().filter(|&b| b == b'\n').count()).unwrap_or(u64::MAX);
        words.saturating_add(newlines.saturating_mul(NEWLINE_THIRTIETHS).div_ceil(30))
    }
}

/// Per-newline cost in thirtieths of a token (half a token).
const NEWLINE_THIRTIETHS: u64 = 15;

/// The ceiling of one word's summed per-char costs.
fn word_cost(word: &str) -> u64 {
    let thirtieths = word
        .chars()
        .map(|ch| {
            if is_cjk(ch) {
                CJK_THIRTIETHS
            } else if ch.is_ascii_digit() {
                DIGIT_THIRTIETHS
            } else {
                OTHER_THIRTIETHS
            }
        })
        .fold(0, u64::saturating_add);
    thirtieths.div_ceil(30)
}

/// Hiragana/Katakana, CJK Unified Ideographs (+ ext. A), Hangul syllables,
/// and CJK compatibility ideographs — the scripts that tokenize to roughly
/// one token per char.
fn is_cjk(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x3040..=0x30FF | 0x3400..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
    )
}

/// Header-only image dimension sniff + a single token-cost formula for
/// inline media fragments (US-009, PR1: images only).
///
/// This is deliberately *not* a [`TokenEstimator`] impl: it does not take
/// text, it takes a mime and raw decoded bytes, and its cost model has
/// nothing in common with the char-class heuristic. It parses just enough of
/// PNG (the `IHDR` chunk) and JPEG (the first `SOF0`/`SOF2` marker) to read
/// pixel dimensions — no other chunks/markers, no color data, no CRC
/// verification. The cost formula (`ceil(width * height / 750)`) is Claude's
/// published image-token constant; picking one formula at launch is a
/// deliberate simplification — a per-provider cost model is a documented
/// future seam, not built here.
///
/// Any mime this module does not recognize, or bytes whose header cannot be
/// read (too short, bad signature, no `SOF` marker found), fall back to the
/// crate's default text estimator run over `bytes_b64` — the base64 text is
/// always longer than a tight token count would be, so this is a safe
/// over-count, never a silent under-count of a real image's cost.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageTokenEstimator;

/// Claude's published pixels-per-token constant for image inputs.
const CLAUDE_PIXELS_PER_TOKEN: u64 = 750;

impl ImageTokenEstimator {
    /// Estimated prompt-token cost of one image fragment. `bytes` are the
    /// *decoded* raw media; `bytes_b64` is the original base64 text, read
    /// only by the fallback path.
    #[must_use]
    pub fn estimate(mime: &str, bytes: &[u8], bytes_b64: &str) -> u64 {
        match image_dimensions(mime, bytes) {
            Some((width, height)) => {
                let pixels = u64::from(width).saturating_mul(u64::from(height));
                pixels.div_ceil(CLAUDE_PIXELS_PER_TOKEN)
            }
            None => HeuristicEstimator.estimate(bytes_b64),
        }
    }
}

/// Sniff pixel dimensions from a mime-tagged image payload, or `None` when
/// the mime is unsupported or the header cannot be parsed.
fn image_dimensions(mime: &str, bytes: &[u8]) -> Option<(u32, u32)> {
    match mime {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" | "image/jpg" => jpeg_dimensions(bytes),
        _ => None,
    }
}

/// Read `(width, height)` from a PNG's leading `IHDR` chunk. Bounds-checked
/// throughout (`slice::get`, never a panicking index) — a truncated or
/// corrupt payload yields `None`, never a panic, since this runs over
/// caller-controlled bytes.
fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.get(0..8)? != SIGNATURE {
        return None;
    }
    // Chunk layout: 4-byte length, 4-byte type ("IHDR" for the first chunk),
    // then IHDR's own data: 4-byte width, 4-byte height (both big-endian),
    // followed by bit depth/color type/etc. (unread here).
    if bytes.get(12..16)? != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    // A forged zero dimension would price a multi-MiB payload at 0 tokens —
    // a silent under-count. Treat it as unparseable: the caller falls back
    // to the safe over-counting text estimate.
    if width == 0 || height == 0 {
        return None;
    }
    Some((width, height))
}

/// Read `(width, height)` from a JPEG's first baseline (`SOF0`, `0xC0`) or
/// progressive (`SOF2`, `0xC2`) marker segment, walking past any other
/// marker segment (`APPn`, `DQT`, `DHT`, …) that precedes it. Bounds-checked
/// throughout; a truncated, malformed, or `SOF`-less stream yields `None`.
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SOF0: u8 = 0xC0;
    const SOF2: u8 = 0xC2;
    if bytes.get(0..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2_usize;
    while let Some(&marker_byte) = bytes.get(pos) {
        if marker_byte != 0xFF {
            return None;
        }
        let marker = *bytes.get(pos + 1)?;
        // Standalone markers (RSTn, and the SOI/EOI we may re-encounter)
        // carry no length or payload.
        if (0xD0..=0xD9).contains(&marker) {
            pos += 2;
            continue;
        }
        let seg_len = usize::from(u16::from_be_bytes(
            bytes.get(pos + 2..pos + 4)?.try_into().ok()?,
        ));
        if seg_len < 2 {
            return None;
        }
        if marker == SOF0 || marker == SOF2 {
            // Payload: 1-byte precision, 2-byte height, 2-byte width
            // (both big-endian) — component data follows, unread here.
            let payload = bytes.get(pos + 4..pos + 9)?;
            let height = u16::from_be_bytes([payload[1], payload[2]]);
            let width = u16::from_be_bytes([payload[3], payload[4]]);
            // height == 0 is legal JPEG (DNL-deferred) but unpriceable, and
            // width == 0 is forged either way: fall back to the safe
            // over-counting text estimate rather than a 0-token under-count.
            if width == 0 || height == 0 {
                return None;
            }
            return Some((u32::from(width), u32::from(height)));
        }
        pos += 2 + seg_len;
    }
    None
}

#[cfg(test)]
#[path = "estimator_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "image_estimator_tests.rs"]
mod image_estimator_tests;
