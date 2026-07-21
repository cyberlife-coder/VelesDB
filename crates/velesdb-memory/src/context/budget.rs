//! Priority packing under a token budget.
//!
//! Invariant: the sum of per-piece token estimates of everything packed,
//! plus one joiner token per packed fragment, never exceeds the usable
//! budget. For a superadditive estimator (the default char-ratio one rounds
//! every piece up) this bounds the estimate of the assembled text, so the
//! output provably fits.
//!
//! Selection is a deterministic total order — critical first, then caller
//! priority, then a cache/non-cache tier, then relevance (non-cache items
//! only), then input order — with `seq` as the final tie-break so equal
//! fragments can never swap places between runs.
//!
//! **Trade-off (issue #1455): cache stability over relevance, for
//! cache-marked fragments only.** A `cache: true` fragment (the
//! `cache.stable_prefix` classification) forms the provider prompt-cache
//! prefix, whose entire value is being byte-identical across turns. Ranking
//! it by lexical relevance to the query — like every other fragment — would
//! let a query change alone decide which of two same-priority cache
//! fragments wins a tight budget, silently changing the prefix's bytes and
//! defeating the provider cache on exactly the turn a new question is
//! asked. So a cache-marked fragment's rank never consults relevance, in
//! either direction: it always outranks a non-cache fragment of the same
//! criticality/priority (a fixed, query-independent tier — see
//! [`selection_order`]), and two cache-marked fragments tied on priority
//! fall straight to `seq`. Non-cache fragments are unaffected: relevance
//! remains their tie-break, exactly as before. The accepted cost: a
//! more-relevant non-cache fragment can lose a tight-budget race it would
//! have won pre-#1455 against a same-tier cache fragment.

use super::estimator::TokenEstimator;

/// The separator emitted between packed fragments — the single source both
/// the packing accountant and the assembly joins use, so the accounted cost
/// and the emitted bytes can never drift apart.
pub(crate) const JOINER: &str = "\n\n";

/// One emission piece: its text, and — for a piece whose token cost is
/// precomputed outside the injected estimator (a media fragment's single
/// atomic piece, US-009 PR1) — that fixed cost. `None` (every non-media
/// piece) means "measure `text` with the injected estimator", the unchanged
/// pre-media behavior; `Some` must never be re-derived from `text` (for
/// media, `text` is only the caption, which on its own would grossly
/// under-price the piece).
#[derive(Debug, Clone)]
pub(crate) struct Piece {
    pub text: String,
    pub cost: Option<u64>,
}

/// One packable fragment: its emission pieces plus its selection keys.
#[derive(Debug)]
pub(crate) struct PackItem {
    /// Input position — the last tie-break, and the emission order.
    pub seq: usize,
    /// Critical fragments pack before everything else.
    pub critical: bool,
    /// Caller priority (higher first).
    pub priority: u8,
    /// Lexical relevance to the query (higher first). Never consulted for a
    /// `cache`-marked item — see the module trade-off note and
    /// [`selection_order`].
    pub relevance: f32,
    /// Whether this fragment classified as `cache.stable_prefix` (the
    /// provider prompt-cache prefix). Cache-marked items rank ahead of
    /// non-cache items at the same criticality/priority, and break ties
    /// among themselves on `seq` alone — never on `relevance` — so their
    /// selection is fully query-independent (issue #1455).
    pub cache: bool,
    /// The fragment's text, pre-cut into orderly pieces (chunks); packing
    /// takes a prefix of them.
    pub pieces: Vec<Piece>,
}

/// How many leading pieces of each item fit under `usable` tokens. The
/// result is aligned with `items` (input order), not with selection order.
pub(crate) fn pack(items: &[PackItem], usable: u64, estimator: &dyn TokenEstimator) -> Vec<usize> {
    let mut taken = vec![0_usize; items.len()];
    let mut remaining = usable;
    // Priced by the *injected* estimator, not a constant: a tokenizer that
    // prices "\n\n" higher than the default would otherwise overflow the
    // budget by one under-counted joiner per fragment.
    let joiner_tokens = estimator.estimate(JOINER);
    for &index in &selection_order(items) {
        let item = &items[index];
        taken[index] = take_pieces(&item.pieces, &mut remaining, joiner_tokens, estimator);
    }
    taken
}

/// Item indices in packing order: critical desc, priority desc, cache
/// (cache-marked before non-cache) desc, relevance desc (non-cache items
/// only — see the module trade-off note), seq asc.
///
/// The cache tier is decided purely from each item's own `cache` flag, never
/// from `relevance`, so it can never be perturbed by a query change: two
/// cache-marked items always tie it (falling to `seq`), and a cache-marked
/// item always beats a non-cache one at the same criticality/priority. Only
/// once both sides of a comparison are confirmed non-cache does `relevance`
/// enter at all — the query can reorder non-cache fragments among
/// themselves exactly as before, but can never move a cache-marked fragment
/// relative to anything else.
fn selection_order(items: &[PackItem]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let (left, right) = (&items[a], &items[b]);
        right
            .critical
            .cmp(&left.critical)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| right.cache.cmp(&left.cache))
            .then_with(|| {
                if left.cache {
                    // Both cache-marked (the tie-break above matched): never
                    // consult relevance — fall straight through to `seq`.
                    std::cmp::Ordering::Equal
                } else {
                    right.relevance.total_cmp(&left.relevance)
                }
            })
            .then_with(|| left.seq.cmp(&right.seq))
    });
    order
}

/// Greedily take leading pieces while they fit; the first piece also pays
/// the fragment's joiner cost. A piece's own cost is its precomputed
/// [`Piece::cost`] when set, otherwise the injected estimator over its text.
fn take_pieces(
    pieces: &[Piece],
    remaining: &mut u64,
    joiner_tokens: u64,
    estimator: &dyn TokenEstimator,
) -> usize {
    let mut count = 0_usize;
    for piece in pieces {
        let joiner = if count == 0 { joiner_tokens } else { 0 };
        let base = piece
            .cost
            .unwrap_or_else(|| estimator.estimate(&piece.text));
        let cost = base.saturating_add(joiner);
        if cost > *remaining {
            break;
        }
        *remaining -= cost;
        count += 1;
    }
    count
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
