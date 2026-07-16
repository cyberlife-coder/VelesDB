//! Priority packing under a token budget.
//!
//! Invariant: the sum of per-piece token estimates of everything packed,
//! plus one joiner token per packed fragment, never exceeds the usable
//! budget. For a superadditive estimator (the default char-ratio one rounds
//! every piece up) this bounds the estimate of the assembled text, so the
//! output provably fits.
//!
//! Selection is a deterministic total order — critical first, then caller
//! priority, then relevance, then input order — with `seq` as the final
//! tie-break so equal fragments can never swap places between runs.

use super::estimator::TokenEstimator;

/// Cost accounted for the `\n\n` joiner between packed fragments.
const JOINER_TOKENS: u64 = 1;

/// One packable fragment: its emission pieces plus its selection keys.
#[derive(Debug)]
pub(crate) struct PackItem {
    /// Input position — the last tie-break, and the emission order.
    pub seq: usize,
    /// Critical fragments pack before everything else.
    pub critical: bool,
    /// Caller priority (higher first).
    pub priority: u8,
    /// Lexical relevance to the query (higher first).
    pub relevance: f32,
    /// The fragment's text, pre-cut into orderly pieces (chunks); packing
    /// takes a prefix of them.
    pub pieces: Vec<String>,
}

/// How many leading pieces of each item fit under `usable` tokens. The
/// result is aligned with `items` (input order), not with selection order.
pub(crate) fn pack(items: &[PackItem], usable: u64, estimator: &dyn TokenEstimator) -> Vec<usize> {
    let mut taken = vec![0_usize; items.len()];
    let mut remaining = usable;
    for &index in &selection_order(items) {
        let item = &items[index];
        taken[index] = take_pieces(&item.pieces, &mut remaining, estimator);
    }
    taken
}

/// Item indices in packing order: critical desc, priority desc, relevance
/// desc, seq asc.
fn selection_order(items: &[PackItem]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        let (left, right) = (&items[a], &items[b]);
        right
            .critical
            .cmp(&left.critical)
            .then_with(|| right.priority.cmp(&left.priority))
            .then_with(|| right.relevance.total_cmp(&left.relevance))
            .then_with(|| left.seq.cmp(&right.seq))
    });
    order
}

/// Greedily take leading pieces while they fit; the first piece also pays
/// the fragment's joiner token.
fn take_pieces(pieces: &[String], remaining: &mut u64, estimator: &dyn TokenEstimator) -> usize {
    let mut count = 0_usize;
    for piece in pieces {
        let joiner = if count == 0 { JOINER_TOKENS } else { 0 };
        let cost = estimator.estimate(piece).saturating_add(joiner);
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
