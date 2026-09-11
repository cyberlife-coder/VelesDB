//! Reachability anchors: one base-layer edge per node that eviction never
//! removes (#2259).
//!
//! HNSW links a new node to its neighbours, and each neighbour back to it,
//! evicting from a full list to make room. An eviction that takes a node's
//! last in-edge leaves it stored and mapped but out of reach of every search,
//! whatever its `ef`: parallel batches lost 5.6 % of a 4-D curve's nodes that
//! way, and 1.7 % of 100 000 random 128-D vectors.
//!
//! So every node gets an *anchor*: an already-anchored node, or the entry
//! point, whose base-layer list holds an edge to it that eviction skips.
//! Anchors form a tree rooted at the entry point — a promoted entry point
//! drops its own anchor and takes the previous one into its subtree — so every
//! anchored node is reachable from it over the base layer, along protected
//! edges. Reachable is what the anchors guarantee; a search, which starts
//! from where its greedy descent lands and stops on stagnation, still walks
//! the graph approximately.

use super::super::distance::DistanceEngine;
use super::super::layer::{Layer, NodeId};
use super::{NativeHnsw, NO_ENTRY_POINT};
use crate::perf_optimizations::ContiguousVectors;
use std::collections::VecDeque;
use std::sync::atomic::Ordering;

/// How far down the anchor tree [`NativeHnsw::link_protected`] looks for a
/// list with room before growing one past its cap. A descent continues only
/// through full lists whose every entry is protected, and descents spread over
/// those protected children (`spread_protected_child`) instead of all taking
/// the first. No tighter bound is proven; on 3,000 exact duplicates no descent
/// reaches this one (`duplicates_stay_reachable_without_any_list_growing_past_its_cap`).
const MAX_ANCHOR_DESCENT: usize = 32;

/// Where [`NativeHnsw::link_protected`] placed the edge, or where it goes
/// next.
enum Placement {
    Linked,
    Descend(NodeId),
}

impl<D: DistanceEngine> NativeHnsw<D> {
    /// Gives `node` its anchor: the first node of its own base-layer list
    /// that is anchored, or is the entry point, takes a protected edge to it.
    ///
    /// Returns `false` when no such neighbour exists yet — batch mates still
    /// connecting; [`Self::anchor_pending`] retries once they are anchored.
    pub(in crate::index::hnsw::native) fn anchor_node(
        &self,
        node: NodeId,
        vectors: &ContiguousVectors,
        layers: &[Layer],
    ) -> bool {
        let base = &layers[0];
        debug_assert!(base.records_anchors(), "layer 0 must record anchors");
        let entry_point = self.entry_point.load(Ordering::Acquire);
        if node == entry_point || base.anchor_of(node).is_some() {
            return true;
        }
        let parent = base
            .with_neighbors(node, |list| {
                list.iter()
                    .copied()
                    .find(|&n| n == entry_point || base.anchor_of(n).is_some())
            })
            .flatten();
        parent.is_some_and(|parent| self.link_protected(parent, node, vectors, base))
    }

    /// Makes a base-layer list hold `child` and records that edge as `child`'s
    /// anchor, both under the list's lock: `parent`'s list, or one below it.
    /// Returns `false` when the list does not exist.
    ///
    /// A full list gives up its farthest entry not anchored to its owner: that
    /// entry's anchor, once it has one, is another list's edge. When every
    /// entry of a full list is protected, the edge goes one level down the
    /// anchor tree instead, to a protected child picked by
    /// [`Self::spread_protected_child`], since any anchored node is a valid
    /// parent. No
    /// list then grows past its cap, where exact duplicates, which never
    /// displace an equal neighbour, used to pile up on one node without bound.
    /// Only past [`MAX_ANCHOR_DESCENT`] levels does a list grow by one.
    pub(in crate::index::hnsw::native) fn link_protected(
        &self,
        parent: NodeId,
        child: NodeId,
        vectors: &ContiguousVectors,
        base: &Layer,
    ) -> bool {
        let mut owner = parent;
        for depth in 0..=MAX_ANCHOR_DESCENT {
            let placement = base.with_neighbors_mut(owner, |list| {
                if !list.contains(&child) {
                    if list.len() >= self.max_connections_0 {
                        match self.farthest_unprotected(owner, list, vectors, base) {
                            Some(evict) => {
                                list.swap_remove(evict);
                            }
                            None if depth < MAX_ANCHOR_DESCENT => {
                                if let Some(next) =
                                    Self::spread_protected_child(owner, list, child, base)
                                {
                                    return Placement::Descend(next);
                                }
                            }
                            None => {}
                        }
                    }
                    list.push(child);
                }
                base.set_anchor(child, owner);
                Placement::Linked
            });
            match placement {
                Some(Placement::Linked) => return true,
                Some(Placement::Descend(next)) => owner = next,
                None => return false,
            }
        }
        false
    }

    /// The entry of `owner`'s list anchored to `owner` where
    /// [`Self::link_protected`] goes when every entry of a full list is
    /// protected, picked by a hash of the entry and `child`. Successive
    /// descents then spread over the protected children, and the chain of full
    /// lists a descent follows stays short, as [`MAX_ANCHOR_DESCENT`] assumes.
    /// The closest entry ties on equal distances instead: exact duplicates all
    /// took the first child, the chain grew to the bound, and the list at its
    /// end grew past its cap (#2259).
    fn spread_protected_child(
        owner: NodeId,
        list: &[NodeId],
        child: NodeId,
        base: &Layer,
    ) -> Option<NodeId> {
        let mix = |n: NodeId| {
            (n as u64 ^ (child as u64).rotate_left(32)).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        };
        list.iter()
            .copied()
            .filter(|&n| base.anchor_of(n) == Some(owner))
            .min_by_key(|&n| mix(n))
    }

    /// Index of the entry of `owner`'s list farthest from `owner` that is not
    /// some node's anchor edge, if there is one.
    fn farthest_unprotected(
        &self,
        owner: NodeId,
        list: &[NodeId],
        vectors: &ContiguousVectors,
        base: &Layer,
    ) -> Option<usize> {
        let owner_vec = vectors.get(owner)?;
        list.iter()
            .enumerate()
            .filter(|&(_, &n)| base.anchor_of(n) != Some(owner))
            .filter_map(|(i, &n)| {
                vectors
                    .get(n)
                    .map(|v| (i, self.distance.distance(owner_vec, v)))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Anchors every node of `pending`, retrying until a pass adds no anchor —
    /// each pass can anchor the parent the next one needs — then links what
    /// is left from the entry point.
    pub(in crate::index::hnsw::native) fn anchor_pending(&self, mut pending: Vec<NodeId>) {
        if pending.is_empty() {
            return;
        }
        self.with_vectors_and_layers_read(|vectors, layers| {
            loop {
                let before = pending.len();
                pending.retain(|&node| !self.anchor_node(node, vectors, layers));
                if pending.is_empty() || pending.len() == before {
                    break;
                }
            }
            for node in pending {
                self.link_from_entry_point(node, vectors, layers);
            }
        });
    }

    /// [`Self::anchor_pending`] for the single node an insert places, without
    /// allocating.
    pub(in crate::index::hnsw::native) fn anchor_one(&self, node: NodeId) {
        self.with_vectors_and_layers_read(|vectors, layers| {
            if !self.anchor_node(node, vectors, layers) {
                self.link_from_entry_point(node, vectors, layers);
            }
        });
    }

    /// The last resort for a node no anchored neighbour can take: a protected
    /// edge from the entry point, or from below it.
    fn link_from_entry_point(&self, node: NodeId, vectors: &ContiguousVectors, layers: &[Layer]) {
        let entry_point = self.entry_point.load(Ordering::Acquire);
        if entry_point != NO_ENTRY_POINT && node != entry_point {
            self.link_protected(entry_point, node, vectors, &layers[0]);
        }
    }

    /// Keeps the anchor tree rooted at the entry point when it moves: the new
    /// entry point, the root, drops the anchor it had as an ordinary node and
    /// links the previous one into its subtree with a protected edge: from its
    /// own list, or one below it when that list is full of protected entries.
    ///
    /// Runs under the promotion lock, in the same step as the swap
    /// ([`Self::promote_entry_point`]): two promotions' reparentings never
    /// interleave, so none clears an anchor another has just given.
    pub(in crate::index::hnsw::native) fn reparent_entry_point(
        &self,
        previous: NodeId,
        new_entry: NodeId,
    ) {
        if previous == NO_ENTRY_POINT || previous == new_entry {
            return;
        }
        self.with_vectors_and_layers_read(|vectors, layers| {
            layers[0].clear_anchor(new_entry);
            self.link_protected(new_entry, previous, vectors, &layers[0]);
        });
    }

    /// Rebuilds every anchor from the base layer as it stands, as a
    /// breadth-first tree from the entry point, then anchors each node that
    /// walk missed.
    ///
    /// Anchors are not persisted: a loaded graph recovers them here. A graph
    /// saved before #2259 can hold nodes nothing links to any more. Each still
    /// lists its own neighbours, so it is anchored from them, as a batch
    /// anchors a node its mates could not, and from the entry point failing
    /// that. An empty list marks a slot no node was linked into, and it stays
    /// out.
    pub(in crate::index::hnsw::native) fn rebuild_anchors(&self) {
        let layers = self.layers.read();
        let Some(base) = layers.first() else {
            return;
        };
        let reached = self.walk_base_from_entry_point(base, |parent, node| {
            base.set_anchor(node, parent);
        });
        let lost: Vec<NodeId> = reached
            .iter()
            .enumerate()
            .filter(|&(node, &seen)| {
                !seen && base.with_neighbors(node, |list| !list.is_empty()) == Some(true)
            })
            .map(|(node, _)| node)
            .collect();
        drop(layers);
        self.anchor_pending(lost);
    }

    /// Walks the base layer breadth-first from the entry point, reporting each
    /// node reached (the entry point aside) with the node it was reached
    /// from, and returns which slots were reached.
    fn walk_base_from_entry_point(
        &self,
        base: &Layer,
        mut reached: impl FnMut(NodeId, NodeId),
    ) -> Vec<bool> {
        let entry_point = self.entry_point.load(Ordering::Acquire);
        let capacity = base.neighbors.len();
        let mut seen = vec![false; capacity];
        if entry_point >= capacity {
            return seen;
        }
        seen[entry_point] = true;
        let mut queue = VecDeque::from([entry_point]);
        while let Some(node) = queue.pop_front() {
            let _ = base.with_neighbors(node, |list| {
                for &next in list {
                    if next < capacity && !seen[next] {
                        seen[next] = true;
                        reached(node, next);
                        queue.push_back(next);
                    }
                }
            });
        }
        seen
    }

    /// The nodes among `nodes` that no base-layer walk from the entry point
    /// reaches.
    #[cfg(test)]
    #[must_use]
    fn unreachable_among(&self, nodes: impl IntoIterator<Item = NodeId>) -> Vec<NodeId> {
        let layers = self.layers.read();
        let Some(base) = layers.first() else {
            return nodes.into_iter().collect();
        };
        let reached = self.walk_base_from_entry_point(base, |_, _| {});
        drop(layers);
        nodes
            .into_iter()
            .filter(|&node| !reached.get(node).copied().unwrap_or(false))
            .collect()
    }
}

#[cfg(test)]
#[path = "anchors_tests.rs"]
mod anchors_tests;
