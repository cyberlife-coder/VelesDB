//! Unit tests for budget packing.

use super::*;
use crate::context::estimator::HeuristicEstimator;

fn item(seq: usize, critical: bool, priority: u8, relevance: f32, pieces: &[&str]) -> PackItem {
    cache_item(seq, critical, priority, relevance, false, pieces)
}

/// Like [`item`], but with an explicit `cache` flag — for tests exercising
/// the cache/non-cache selection tier (issue #1455).
fn cache_item(
    seq: usize,
    critical: bool,
    priority: u8,
    relevance: f32,
    cache: bool,
    pieces: &[&str],
) -> PackItem {
    PackItem {
        seq,
        critical,
        priority,
        relevance,
        cache,
        pieces: pieces
            .iter()
            .map(|p| Piece {
                text: (*p).to_owned(),
                cost: None,
            })
            .collect(),
    }
}

/// A single atomic piece with a precomputed cost, ignoring its text's own
/// estimated cost entirely — the media packing shape (US-009, PR1).
fn media_item(seq: usize, critical: bool, precomputed_cost: u64) -> PackItem {
    PackItem {
        seq,
        critical,
        priority: 0,
        relevance: 0.0,
        cache: false,
        pieces: vec![Piece {
            text: String::new(),
            cost: Some(precomputed_cost),
        }],
    }
}

/// `abcde` estimates to exactly 2 tokens under the heuristic estimator, so a
/// piece costs 2 (+1 joiner on the first piece of an item).
const PIECE: &str = "abcde";

#[test]
fn test_pack_takes_everything_when_budget_is_generous() {
    let items = vec![
        item(0, false, 0, 0.0, &[PIECE]),
        item(1, false, 0, 0.0, &[PIECE]),
    ];
    let taken = pack(&items, 1_000, &HeuristicEstimator);
    assert_eq!(taken, vec![1, 1]);
}

#[test]
fn test_pack_respects_the_budget_strictly() {
    // Each item costs 2 + 1 = 3 tokens; a budget of 5 holds only one.
    let items = vec![
        item(0, false, 0, 0.0, &[PIECE]),
        item(1, false, 0, 0.0, &[PIECE]),
    ];
    let taken = pack(&items, 5, &HeuristicEstimator);
    assert_eq!(taken.iter().sum::<usize>(), 1);
}

#[test]
fn test_pack_critical_items_pack_before_higher_relevance_prose() {
    let items = vec![
        item(0, false, 0, 1.0, &[PIECE]),
        item(1, true, 0, 0.0, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(taken, vec![0, 1], "the critical item must win the budget");
}

#[test]
fn test_pack_priority_orders_within_same_criticality() {
    let items = vec![
        item(0, false, 1, 0.0, &[PIECE]),
        item(1, false, 9, 0.0, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(taken, vec![0, 1], "priority 9 must pack first");
}

#[test]
fn test_pack_relevance_breaks_priority_ties() {
    let items = vec![
        item(0, false, 0, 0.2, &[PIECE]),
        item(1, false, 0, 0.8, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(taken, vec![0, 1], "higher relevance must pack first");
}

#[test]
fn test_pack_seq_breaks_full_ties_deterministically() {
    let items = vec![
        item(0, false, 0, 0.5, &[PIECE]),
        item(1, false, 0, 0.5, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(taken, vec![1, 0], "earlier input order must win ties");
}

#[test]
fn test_pack_takes_a_prefix_of_pieces_when_only_part_fits() {
    // 3 pieces of 2 tokens each + 1 joiner = 7 total; a budget of 5 fits the
    // joiner and two pieces.
    let items = vec![item(0, false, 0, 0.0, &[PIECE, PIECE, PIECE])];
    let taken = pack(&items, 5, &HeuristicEstimator);
    assert_eq!(taken, vec![2]);
}

#[test]
fn test_pack_zero_budget_takes_nothing() {
    let items = vec![item(0, true, 9, 1.0, &[PIECE])];
    assert_eq!(pack(&items, 0, &HeuristicEstimator), vec![0]);
}

#[test]
fn test_pack_empty_items_yields_empty_result() {
    assert!(pack(&[], 100, &HeuristicEstimator).is_empty());
}

#[test]
fn test_pack_uses_a_piece_precomputed_cost_instead_of_estimating_its_text() {
    // The piece's text is empty (would estimate to 0 tokens); its
    // precomputed cost of 900 must still be what's charged against the
    // budget — proving `take_pieces` never falls back to `estimator.estimate`
    // when `Piece::cost` is set.
    let items = vec![media_item(0, false, 900)];
    // Budget covers the precomputed cost (900) + 1 joiner token, not more.
    let joiner = HeuristicEstimator.estimate(JOINER);
    let taken = pack(&items, 900 + joiner, &HeuristicEstimator);
    assert_eq!(
        taken,
        vec![1],
        "the precomputed-cost piece must fit exactly"
    );
    let too_tight = pack(&items, 900 + joiner - 1, &HeuristicEstimator);
    assert_eq!(
        too_tight,
        vec![0],
        "one token under the precomputed cost must not fit"
    );
}

#[test]
fn test_pack_precomputed_cost_piece_is_atomic_never_partially_taken() {
    // A media item always has exactly one piece, so `taken` can only ever be
    // 0 or 1 — there is no partial state to assert against, but this pins
    // that a too-small budget takes nothing rather than some fractional
    // amount.
    let items = vec![media_item(0, true, 1_000_000)];
    let taken = pack(&items, 10, &HeuristicEstimator);
    assert_eq!(taken, vec![0]);
}

// === Cache selection tier (issue #1455) =====================================
//
// A cache-marked item's rank must never consult `relevance` — neither
// against another cache item nor against a non-cache one — so two
// same-priority cache items always resolve on `seq` alone, and a cache item
// always beats a same-tier non-cache item regardless of how relevant that
// non-cache item is to the (here, simulated by a raw `relevance` field)
// query.

#[test]
fn test_pack_two_cache_items_ignore_relevance_and_break_ties_on_seq() {
    // Same critical/priority, cache-marked, but relevance strongly favors
    // item 1 — if relevance were consulted, item 1 would win. It must not.
    let items = vec![
        cache_item(0, true, 0, 0.1, true, &[PIECE]),
        cache_item(1, true, 0, 0.9, true, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(
        taken,
        vec![1, 0],
        "earlier seq must win between two cache items regardless of relevance"
    );
}

#[test]
fn test_pack_cache_item_beats_higher_relevance_non_cache_item_at_same_tier() {
    // Item 0 is non-cache with high relevance; item 1 is cache-marked with
    // low relevance. Pre-#1455 behavior would let relevance decide (item 0
    // wins); the fix requires the cache item to win instead, so the cache
    // prefix never depends on a competing non-cache fragment's relevance.
    let items = vec![
        cache_item(0, true, 0, 0.9, false, &[PIECE]),
        cache_item(1, true, 0, 0.1, true, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(
        taken,
        vec![0, 1],
        "the cache-marked item must win the tight budget over a more relevant non-cache item"
    );
}

#[test]
fn test_pack_non_cache_items_still_use_relevance_unaffected_by_cache_tier() {
    // No cache items in play at all: behavior must be byte-identical to
    // pre-#1455 — relevance still breaks priority ties among non-cache
    // fragments.
    let items = vec![
        cache_item(0, false, 0, 0.2, false, &[PIECE]),
        cache_item(1, false, 0, 0.8, false, &[PIECE]),
    ];
    let taken = pack(&items, 3, &HeuristicEstimator);
    assert_eq!(taken, vec![0, 1], "higher relevance must still pack first");
}
