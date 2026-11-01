//! `SemanticMemory`'s relation accessors — a `#[path]` child of
//! `semantic_memory.rs` that keeps it under the file budget. Each read
//! delegates to `memory_helpers`, which consults the database's read gate
//! before it reads the graph (`AgentMemory` reads are collection reads).

use super::super::error::AgentMemoryError;
use super::super::memory_helpers;
use super::super::ttl::MemoryKind;
use super::SemanticMemory;

impl SemanticMemory {
    /// Returns the outgoing relations of a fact (edges it points from).
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn relations(
        &self,
        id: u64,
    ) -> Result<Vec<crate::collection::graph::GraphEdge>, AgentMemoryError> {
        memory_helpers::relations_of(
            &self.db,
            &self.collection_name,
            id,
            &self.ttl,
            MemoryKind::Semantic,
        )
    }

    /// Returns the incoming relations of a fact (edges pointing at it).
    ///
    /// The mirror of [`Self::relations`]: edges whose *source* is TTL-expired
    /// are filtered out.
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn incoming_relations(
        &self,
        id: u64,
    ) -> Result<Vec<crate::collection::graph::GraphEdge>, AgentMemoryError> {
        memory_helpers::incoming_relations_of(
            &self.db,
            &self.collection_name,
            id,
            &self.ttl,
            MemoryKind::Semantic,
        )
    }

    /// Returns at most `cap` outgoing relations of a fact, plus whether its
    /// total degree exceeded the scan — work and transient allocation
    /// O(cap), never O(degree) (#1820).
    ///
    /// Prefer this over [`Self::relations`] wherever the caller keeps only a
    /// bounded prefix: on a super-node (an entity hub mentioned by thousands
    /// of facts) the unbounded accessor materializes the whole degree before
    /// the caller's own cap can apply.
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn relations_bounded(
        &self,
        id: u64,
        cap: usize,
    ) -> Result<crate::agent::BoundedRelations, AgentMemoryError> {
        memory_helpers::relations_of_bounded(
            &self.db,
            &self.collection_name,
            id,
            &self.ttl,
            MemoryKind::Semantic,
            cap,
        )
    }

    /// Returns at most `cap` incoming relations of a fact, plus whether its
    /// total incoming degree exceeded the scan — the mirror of
    /// [`Self::relations_bounded`].
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn incoming_relations_bounded(
        &self,
        id: u64,
        cap: usize,
    ) -> Result<crate::agent::BoundedRelations, AgentMemoryError> {
        memory_helpers::incoming_relations_of_bounded(
            &self.db,
            &self.collection_name,
            id,
            &self.ttl,
            MemoryKind::Semantic,
            cap,
        )
    }
}
