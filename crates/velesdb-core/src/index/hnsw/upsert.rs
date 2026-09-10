//! What an upsert leaves behind, and soft delete — shared by `HnswIndex` and
//! `NativeHnswIndex`.
//!
//! Neither index allocates a slot here: the graph's arena hands out every
//! slot, and the mappings follow it through `ShardedMappings::assign` (#2246).

use super::native_inner::NativeHnswInner;
use super::sharded_mappings::ShardedMappings;

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
/// `vacuum` while it re-maps) re-maps under the write guard: run between the
/// two writes, it would map `id` again, or erase the reverse entry of the id
/// that took its slot. As with `Placed`, the borrow ties the call to some
/// graph's guard, not to this index's: each caller keeps an index's guard and
/// its mappings together.
///
/// [`vacuum`]: crate::index::HnswIndex::vacuum
#[inline]
pub(crate) fn soft_delete(mappings: &ShardedMappings, id: u64, _graph: &NativeHnswInner) -> bool {
    mappings.remove(id).is_some()
}
