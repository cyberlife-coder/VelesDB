//! Tests for `search_past_unmapped`: how many walks it runs, and with what.

use super::search_past_unmapped;
use crate::scored_result::ScoredResult;
use std::cell::RefCell;

/// Nodes at or past this number map to no id.
const FIRST_UNMAPPED: usize = 1_000;

/// A walk over nodes `order` (nearest first) that records each `(k, ef)` it
/// is asked for and returns the first `k` of them.
fn recording_walk<'a>(
    order: &'a [usize],
    calls: &'a RefCell<Vec<(usize, usize)>>,
) -> impl Fn(usize, usize) -> Vec<(usize, f32)> + 'a {
    move |k, ef| {
        calls.borrow_mut().push((k, ef));
        order.iter().take(k).map(|&node| (node, 0.0)).collect()
    }
}

/// Maps nodes below `FIRST_UNMAPPED` to themselves, up to `k` of them.
fn live(nodes: &[(usize, f32)], k: usize) -> Vec<ScoredResult> {
    nodes
        .iter()
        .filter(|&&(node, _)| node < FIRST_UNMAPPED)
        .take(k)
        .map(|&(node, d)| ScoredResult::new(node as u64, d))
        .collect()
}

fn ids(results: &[ScoredResult]) -> Vec<u64> {
    results.iter().map(|r| r.id).collect()
}

/// A search that meets no unmapped node runs the walk once, with the
/// caller's own `k`: the walk's probing and oversampling depend on it.
#[test]
fn a_walk_past_no_unmapped_node_runs_once_with_the_callers_k() {
    let order: Vec<usize> = (0..200).collect();
    let calls = RefCell::new(Vec::new());
    let results = search_past_unmapped(10, 64, recording_walk(&order, &calls), live);
    assert_eq!(ids(&results), (0..10).collect::<Vec<u64>>());
    assert_eq!(calls.into_inner(), vec![(10, 64)]);
}

/// Unmapped nodes among the first `k`: one more walk, for the whole pool.
#[test]
fn unmapped_nodes_among_the_first_k_take_one_walk_of_the_pool() {
    let order: Vec<usize> = [FIRST_UNMAPPED, FIRST_UNMAPPED + 1]
        .into_iter()
        .chain(0..200)
        .collect();
    let calls = RefCell::new(Vec::new());
    let results = search_past_unmapped(2, 64, recording_walk(&order, &calls), live);
    assert_eq!(ids(&results), vec![0, 1]);
    assert_eq!(calls.into_inner(), vec![(2, 64), (64, 64)]);
}

/// A pool with every node unmapped widens once, to twice the `ef`.
#[test]
fn a_pool_of_unmapped_nodes_widens_once() {
    let order: Vec<usize> = (FIRST_UNMAPPED..FIRST_UNMAPPED + 100)
        .chain(0..200)
        .collect();
    let calls = RefCell::new(Vec::new());
    let results = search_past_unmapped(1, 64, recording_walk(&order, &calls), live);
    assert_eq!(ids(&results), vec![0]);
    assert_eq!(calls.into_inner(), vec![(1, 64), (64, 64), (128, 128)]);
}

/// A walk that ran out of nodes is not run again: there are no more.
#[test]
fn an_exhausted_walk_is_not_widened() {
    let order: Vec<usize> = vec![FIRST_UNMAPPED, 0];
    let calls = RefCell::new(Vec::new());
    let results = search_past_unmapped(5, 64, recording_walk(&order, &calls), live);
    assert_eq!(ids(&results), vec![0]);
    assert_eq!(calls.into_inner(), vec![(5, 64), (64, 64)]);
}
