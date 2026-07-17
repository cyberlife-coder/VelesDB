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

#[cfg(test)]
#[path = "estimator_tests.rs"]
mod tests;
