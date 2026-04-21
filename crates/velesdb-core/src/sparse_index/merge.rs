//! k-way merge utilities for sparse posting lists.
//!
//! Extracted from `inverted_index.rs` (following the pattern established by
//! `mutable_segment.rs`) so the inverted-index file stays well below the
//! 500 NLOC quality gate.
//!
//! The helpers here consume owned per-segment runs and return a single
//! sorted-by-`doc_id` vector with last-write-wins semantics — callers
//! must snapshot the per-segment data outside any lock scope before
//! invoking [`merge_sorted_runs`].

use super::types::PostingEntry;

/// Merges several `doc_id`-sorted posting runs into a single sorted vector.
///
/// Consumes its inputs and runs without holding any index lock — callers
/// are expected to snapshot the per-segment data via
/// `SparseInvertedIndex::collect_frozen_runs` and
/// `SparseInvertedIndex::collect_mutable_run` first.
///
/// Applies **last-write-wins** semantics when a `doc_id` appears in more
/// than one run: callers place mutable data after frozen data, so an
/// upsert that crossed a segment freeze contributes its mutable weight
/// exactly once instead of double-counting the stale frozen entry when
/// downstream consumers (e.g. `linear_scan_search`, `brute_force_search`)
/// sum posting weights into an accumulator map.
#[inline]
pub(super) fn merge_sorted_runs(
    frozen_runs: Vec<Vec<PostingEntry>>,
    mutable_run: Vec<PostingEntry>,
) -> Vec<PostingEntry> {
    let mut runs: Vec<Vec<PostingEntry>> = frozen_runs;
    if !mutable_run.is_empty() {
        runs.push(mutable_run);
    }
    match runs.len() {
        0 => Vec::new(),
        1 => runs.into_iter().next().unwrap_or_default(),
        _ => k_way_merge(&runs),
    }
}

/// k-way merge of sorted runs with last-write-wins dedup.
///
/// On each iteration picks the smallest `doc_id` across all run heads,
/// then advances **every** cursor whose head matches that `doc_id`,
/// keeping the entry from the last such run. This coalesces
/// duplicate-`doc_id` entries produced by an upsert crossing a segment
/// freeze into a single output entry with the newest weight.
fn k_way_merge(runs: &[Vec<PostingEntry>]) -> Vec<PostingEntry> {
    let total_len: usize = runs.iter().map(Vec::len).sum();
    let mut result: Vec<PostingEntry> = Vec::with_capacity(total_len);
    let mut cursors: Vec<usize> = vec![0; runs.len()];
    while let Some(min_doc_id) = find_min_head_doc_id(runs, &cursors) {
        if let Some(entry) = advance_matching_runs(runs, &mut cursors, min_doc_id) {
            result.push(entry);
        }
    }
    result
}

/// Returns the smallest `doc_id` among the current heads of `runs`,
/// or `None` when every cursor has exhausted its run.
#[inline]
fn find_min_head_doc_id(runs: &[Vec<PostingEntry>], cursors: &[usize]) -> Option<u64> {
    runs.iter()
        .enumerate()
        .filter_map(|(i, run)| run.get(cursors[i]).map(|entry| entry.doc_id))
        .min()
}

/// Advances every cursor whose run-head matches `target_doc_id`, returning
/// the entry from the **last** such run for the last-write-wins dedup
/// documented on [`merge_sorted_runs`].
#[inline]
fn advance_matching_runs(
    runs: &[Vec<PostingEntry>],
    cursors: &mut [usize],
    target_doc_id: u64,
) -> Option<PostingEntry> {
    let mut picked: Option<PostingEntry> = None;
    for (i, run) in runs.iter().enumerate() {
        if run
            .get(cursors[i])
            .is_some_and(|entry| entry.doc_id == target_doc_id)
        {
            picked = Some(run[cursors[i]]);
            cursors[i] += 1;
        }
    }
    picked
}
