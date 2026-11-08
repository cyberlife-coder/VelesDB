//! Flat CSR adjacency snapshot for the HNSW base layer (#2075, PR-A).
//!
//! The write side is unchanged: `Layer::neighbors` stays
//! `Vec<RwLock<Vec<NodeId>>>` and every mutation lands there first. On top of
//! it the base layer publishes an `ArcSwapOption<CsrAdjacency>` — one
//! `offsets: Vec<u32>` (len n+1) plus one contiguous `neighbors: Vec<u32>` —
//! so a hot-path reader walks a plain `&[u32]` instead of paying a per-node
//! read lock plus a scattered heap `Vec` per visit. `u32` is the snapshot's
//! on-disk width already; a node id that does not fit (`> u32::MAX`, the
//! `has_overflow_ids` case) refuses the publish rather than truncating, and
//! the lock path keeps serving.
//!
//! Freshness mirrors `edge_concurrent`'s debounce (issue #905): writers
//! report their real write count into `pending_writes` + `csr_dirty`;
//! readers see a dirty-below-threshold snapshot as absent and fall back to
//! the per-node locks, which stay authoritative at all times. A rebuild is
//! forced where the O(N+E) cost is amortized: end of `insert_batch_parallel`,
//! `remap_ids`, graph load.
//!
//! Anchors stay out of the snapshot on purpose (#2259): `Vec<AtomicU32>` is
//! a separate mutation channel, already 4 B per node.

use arc_swap::ArcSwapOption;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use super::layer::NodeId;

/// Mutations counted toward a CSR rebuild before a reader may serve a stale
/// snapshot. Mirrors `edge_concurrent::CSR_REBUILD_WRITE_THRESHOLD` — kept as
/// a separate constant so this layer's debounce can drift independently.
pub(crate) const CSR_REBUILD_WRITE_THRESHOLD: u64 = 64;

/// Contiguous adjacency snapshot: `neighbors(offsets[i]..offsets[i+1])` is
/// node `i`'s list. All ids are stored `u32`; conversion back to `NodeId`
/// happens at the read site, where `as usize` is free.
#[derive(Debug)]
pub(crate) struct CsrAdjacency {
    offsets: Vec<u32>,
    neighbors: Vec<u32>,
}

impl CsrAdjacency {
    /// Flattens `lists` into one CSR snapshot, or `None` when any stored id
    /// exceeds `u32::MAX` — the overflow case stays on the lock path instead
    /// of publishing a truncated graph.
    pub(crate) fn build(lists: &[RwLock<Vec<NodeId>>]) -> Option<Self> {
        let mut offsets = Vec::with_capacity(lists.len() + 1);
        let mut neighbors = Vec::new();
        offsets.push(0u32);
        for lock in lists {
            {
                let list = lock.read();
                neighbors.reserve(list.len());
                for &id in list.iter() {
                    neighbors.push(u32::try_from(id).ok()?);
                }
            }
            let end = u32::try_from(neighbors.len()).ok()?;
            offsets.push(end);
        }
        Some(Self { offsets, neighbors })
    }

    /// Node `id`'s neighbor slice, `None` when `id` exceeds the snapshot's
    /// coverage (a node appended after the build — the caller falls back to
    /// the authoritative lock path).
    #[inline]
    pub(crate) fn neighbors(&self, node: usize) -> Option<&[u32]> {
        let &start = self.offsets.get(node)?;
        let &end = self.offsets.get(node + 1)?;
        Some(&self.neighbors[start as usize..end as usize])
    }
}

/// A neighbor list resolved either from the CSR snapshot or from the live
/// per-node `Vec` — one reader-facing shape over both storage widths so the
/// search helpers keep a single body.
#[derive(Debug, Clone, Copy)]
pub(crate) enum NeighborSlice<'a> {
    /// `u32` ids from the flat snapshot.
    Csr(&'a [u32]),
    /// `NodeId` (`usize`) ids from the locked per-node list.
    Live(&'a [NodeId]),
}

impl NeighborSlice<'_> {
    /// Iterates every neighbor as `NodeId`.
    #[inline]
    pub(crate) fn iter(&self) -> impl Iterator<Item = NodeId> + '_ {
        let (csr, live): (&[u32], &[NodeId]) = match self {
            Self::Csr(s) => (s, &[]),
            Self::Live(s) => (&[], s),
        };
        csr.iter().map(|&n| n as usize).chain(live.iter().copied())
    }

    /// Number of neighbors in the list.
    #[inline]
    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Csr(s) => s.len(),
            Self::Live(s) => s.len(),
        }
    }

    /// Neighbor at `index`, converted to `NodeId`.
    #[inline]
    pub(crate) fn get(&self, index: usize) -> Option<NodeId> {
        match self {
            Self::Csr(s) => s.get(index).map(|&n| n as usize),
            Self::Live(s) => s.get(index).copied(),
        }
    }
}

/// Dirty/pending state shared by the writer hooks and `csr_snapshot()`.
#[derive(Debug, Default)]
pub(crate) struct CsrState {
    pub(crate) snapshot: ArcSwapOption<CsrAdjacency>,
    pub(crate) dirty: AtomicBool,
    pub(crate) pending_writes: AtomicU64,
    /// Serializes rebuilds so one reader does the O(N+E) work for all.
    pub(crate) build_lock: parking_lot::Mutex<()>,
}

impl CsrState {
    /// Reports `count` real adjacency mutations (#905's lesson: a batch must
    /// report its actual write count, or a bulk load stays under the
    /// threshold forever and the fast path never engages).
    #[inline]
    pub(crate) fn record_writes(&self, count: u64) {
        if count > 0 {
            self.pending_writes.fetch_add(count, Ordering::AcqRel);
            self.dirty.store(true, Ordering::Release);
        }
    }

    /// Publishes a freshly built snapshot and clears the dirty state.
    pub(crate) fn publish(&self, snapshot: CsrAdjacency) {
        self.snapshot.store(Some(Arc::new(snapshot)));
        self.pending_writes.store(0, Ordering::Release);
        self.dirty.store(false, Ordering::Release);
    }

    /// A snapshot safe to serve: present, not dirty, or dirty at the
    /// threshold and rebuilt under `build_lock` by this call. Returns `None`
    /// while a dirty-below-threshold (or never-built) snapshot exists —
    /// readers then take the lock path, which is always correct.
    pub(crate) fn fresh_snapshot(
        &self,
        lists: &[RwLock<Vec<NodeId>>],
    ) -> Option<Arc<CsrAdjacency>> {
        if !self.dirty.load(Ordering::Acquire) {
            return self.snapshot.load_full();
        }
        if self.pending_writes.load(Ordering::Acquire) < CSR_REBUILD_WRITE_THRESHOLD {
            return None;
        }
        let _build = self.build_lock.lock();
        if !self.dirty.load(Ordering::Acquire) {
            return self.snapshot.load_full();
        }
        let snapshot = CsrAdjacency::build(lists)?;
        self.publish(snapshot);
        self.snapshot.load_full()
    }
}
