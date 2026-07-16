//! Unit tests for budget packing.

use super::*;
use crate::context::estimator::HeuristicEstimator;

fn item(seq: usize, critical: bool, priority: u8, relevance: f32, pieces: &[&str]) -> PackItem {
    PackItem {
        seq,
        critical,
        priority,
        relevance,
        pieces: pieces.iter().map(|p| (*p).to_owned()).collect(),
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
