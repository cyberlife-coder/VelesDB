//! [`NativeHnswInner`]'s dispatches for nodes placed in the arena and not
//! linked into the graph yet: the direct writer's slots. Crash recovery finds
//! them ([`NativeHnswInner::unlinked_nodes`], #2246), and the async builder's
//! drain links them where they are ([`NativeHnswInner::link_placed`], #2264).
//! Both callers need `persistence`, and so does this module.

use super::{HnswBackend, NativeHnswInner};

impl NativeHnswInner {
    /// The nodes among `nodes` the graph never linked into layer 0: mapped,
    /// stored, and out of reach of every search. Crash recovery re-indexes
    /// them (#2246).
    pub(crate) fn unlinked_nodes(&self, nodes: impl IntoIterator<Item = usize>) -> Vec<usize> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.unlinked_nodes(nodes),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.unlinked_nodes(nodes),
            HnswBackend::Sq8(sq8) => sq8.inner.unlinked_nodes(nodes),
        }
    }

    /// Links into the graph, where they are, nodes [`Self::place_unlinked`]
    /// placed: the async builder's drain of the direct writer's slots
    /// (#2264). Like `place_unlinked`, it reaches a quantized backend's graph
    /// and not its code store; `upsert_bulk` keeps quantized collections off
    /// the direct writer (`use_v2` in `collection/core/crud_bulk.rs`).
    ///
    /// # Errors
    ///
    /// Returns an error, and links nothing, if the arena does not hold one of
    /// `nodes`.
    pub(crate) fn link_placed(&self, nodes: &[usize]) -> crate::error::Result<()> {
        match &self.backend {
            HnswBackend::Standard(hnsw) => hnsw.link_placed(nodes),
            HnswBackend::RaBitQ(rabitq) => rabitq.inner.link_placed(nodes),
            HnswBackend::Sq8(sq8) => sq8.inner.link_placed(nodes),
        }
    }
}
