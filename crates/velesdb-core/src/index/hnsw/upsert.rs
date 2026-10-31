//! What an upsert leaves behind, and soft delete — shared by `HnswIndex` and
//! `NativeHnswIndex`.
//!
//! Neither index allocates a slot here: the graph's arena hands out every
//! slot, and the mappings follow it through `ShardedMappings::assign` (#2246).

use super::native_inner::NativeHnswInner;
use super::sharded_mappings::ShardedMappings;
use crate::scored_result::ScoredResult;

/// Where an upsert placed an id: the slot now holding its vector, and the
/// slot it held before, if it was already mapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpsertResult {
    /// The slot now holding the id's vector.
    pub idx: usize,
    /// The slot the id held before this upsert, now a tombstone.
    pub old_idx: Option<usize>,
}

/// Soft-deletes a single ID: removes it from the mappings.
///
/// Returns `true` if the ID existed and was removed, `false` if it was
/// already absent. The HNSW graph node itself is left in place — it becomes
/// a tombstone that is filtered out during search via the reverse mapping.
/// Its vector stays in `ContiguousVectors` until [`vacuum`] reclaims it.
///
/// Shared by `HnswIndex::remove` and `NativeHnswIndex::remove` (identical
/// bodies, #448 Group F consolidation).
///
/// `_graph` borrows the index's graph from the guard its caller holds, so the
/// guard outlives both map writes. A renumber (`reorder_for_locality`, or
/// `vacuum` while it re-maps) re-maps under the write guard; one that runs
/// during this delete could map `id` again, or erase the reverse entry of the
/// id that took its slot. As with `Placed`, the
/// borrow proves a borrow of some graph (through its guard, for an index's
/// own), not that it is this index's graph: each caller keeps an index's guard
/// and its mappings together.
///
/// [`vacuum`]: crate::index::HnswIndex::vacuum
#[inline]
pub(crate) fn soft_delete(mappings: &ShardedMappings, id: u64, _graph: &NativeHnswInner) -> bool {
    mappings.remove(id).is_some()
}

/// The widest `ef` a walk widened past nodes no id maps is given: the cap
/// the bitmap search's retry uses too.
const WIDEST_EF: usize = 10_000;

/// A graph search's first `k` results whose node an id still maps to.
///
/// A delete or an upsert leaves the point's old node in the graph, mapped
/// to no id, until `vacuum`. A walk ranks those nodes with the rest, so
/// mapping its `k` nodes and dropping the unmapped ones can leave fewer than
/// `k` — none at all for `k = 1` at a deleted or moved point's old vector.
///
/// `walk(k, ef)` is the graph search, nearest first; `live(nodes, k)` maps
/// up to `k` of `nodes`, skipping unmapped ones. The walk runs as asked
/// first: its `k` steers the walk itself (multi-entry probing, a quantized
/// backend's oversampling), so it is never widened up front. Only when that
/// leaves fewer than `k` results *and* the walk returned a node no id maps
/// does it run again, at most twice:
///
/// 1. asked for its whole `ef` pool (when that is more than `k`), from which
///    the first `k` mapped nodes are kept;
/// 2. if every node of that pool was used and still too few are mapped — a
///    neighbourhood dense with deleted points — once more at twice the `ef`
///    (at most [`WIDEST_EF`]). Past that, `vacuum` is the remedy.
///
/// A search near no deleted or moved point runs one walk, as before.
pub(crate) fn search_past_unmapped(
    k: usize,
    ef: usize,
    walk: impl Fn(usize, usize) -> Vec<(usize, f32)>,
    live: impl Fn(&[(usize, f32)], usize) -> Vec<ScoredResult>,
) -> Vec<ScoredResult> {
    let walked = walk(k, ef);
    let results = live(&walked, k);
    if !cut_short(&walked, &results, k) {
        return results;
    }
    let (results, pool_used) = if ef > k {
        let walked = walk(ef, ef);
        let results = live(&walked, k);
        let pool_used = walked.len() >= ef;
        if !cut_short(&walked, &results, k) {
            return results;
        }
        (results, pool_used)
    } else {
        (results, walked.len() >= k)
    };
    let wide = ef.saturating_mul(2).min(WIDEST_EF);
    if !pool_used || wide <= ef {
        return results;
    }
    let walked = walk(k.max(wide), wide);
    live(&walked, k)
}

/// Whether mapping `walked` left fewer than `k` results because some of its
/// nodes map to no id.
fn cut_short(walked: &[(usize, f32)], results: &[ScoredResult], k: usize) -> bool {
    results.len() < k && walked.len() > results.len()
}

#[cfg(test)]
#[path = "upsert_tests.rs"]
mod tests;
