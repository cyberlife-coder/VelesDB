//! `remember`'s *detailed* half: the `similar` page — the at-most
//! [`SIMILAR_LIMIT`](super::SIMILAR_LIMIT) pre-existing memories nearest to a
//! just-stored fact — and `with_auto_link`'s thresholded `related_to` edges.
//! Split out of `service_graph_wiring.rs` when the budget crossed (#2746):
//! the write path stays there, the read-back-and-rank path lives here. A
//! `#[path]` child of `service`, sharing full access to `MemoryService`'s
//! private fields and methods.

use super::{
    Embedder, FactStore, GraphStore, Link, MemoryError, MemoryService, Metadata, RecallStore,
};

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// [`Self::remember`], additionally returning the `similar` page — the
    /// at-most [`SIMILAR_LIMIT`](super::SIMILAR_LIMIT) pre-existing memories
    /// nearest to the fact, scored by raw similarity, the fact itself and
    /// internal hubs excluded — and, when the service was built with
    /// [`Self::with_auto_link`], a `related_to` edge to each hit at or above
    /// that threshold. The candidate search reuses the embedding the store
    /// pass already computed, so one remember costs one embed, never two.
    ///
    /// # Errors
    /// The errors of [`Self::remember`].
    pub fn remember_detailed(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
    ) -> Result<crate::model::RememberOutcome, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let _generation = self.enter_generation();
        self.remember_outcome(fact, links, metadata, ttl_seconds, None)
    }

    /// [`Self::remember_detailed`]'s core behind the generation gate its
    /// callers already hold, with the reserved `system` entries the anchored
    /// variant resolves (`service_anchor` is `persistence`-only; the shape
    /// itself is not).
    pub(super) fn remember_outcome(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
        system: Option<&Metadata>,
    ) -> Result<crate::model::RememberOutcome, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let (fact_id, embedding) =
            self.remember_embed(fact, links, metadata, ttl_seconds, true, system)?;
        let similar = self.similar_neighbors(&embedding, fact_id)?;
        self.auto_link_similar(fact_id, &similar);
        Ok(crate::model::RememberOutcome { fact_id, similar })
    }

    /// The `similar` half of [`Self::remember_detailed`]: the at-most
    /// [`SIMILAR_LIMIT`](super::SIMILAR_LIMIT) pre-existing memories nearest
    /// to `embedding`. The search inherits `search`'s hub exclusion and one
    /// extra hit covers the just-stored fact itself, so `self` never comes
    /// back and a re-remembered fact does not crowd a real candidate out of
    /// the page.
    pub(super) fn similar_neighbors(
        &self,
        embedding: &[f32],
        exclude: u64,
    ) -> Result<Vec<crate::model::SimilarMemory>, MemoryError>
    where
        S: RecallStore,
    {
        let hits = self.search(embedding, super::SIMILAR_LIMIT + 1, None)?;
        Ok(hits
            .into_iter()
            .filter(|(hit_id, _score, _content)| *hit_id != exclude)
            .take(super::SIMILAR_LIMIT)
            .map(|(hit_id, score, _content)| crate::model::SimilarMemory { id: hit_id, score })
            .collect())
    }

    /// `with_auto_link`'s edges: a `related_to` from the new fact to every
    /// `similar` hit at or above the configured threshold, through the same
    /// `relate` path an explicit `links` entry takes — validation, endpoint
    /// liveness and triplet-derived idempotence included. A failure never
    /// fails the remember: the edge is system-inferred and recoverable by
    /// re-remembering, so it is logged and skipped (the autograph incident's
    /// "never silent" fix, applied to the sibling feature).
    pub(super) fn auto_link_similar(&self, from: u64, similar: &[crate::model::SimilarMemory])
    where
        S: GraphStore,
    {
        let Some(threshold) = self.auto_link else {
            return;
        };
        for hit in similar.iter().filter(|hit| hit.score >= threshold) {
            if let Err(error) = self.relate_inner(from, hit.id, super::AUTO_LINK_RELATION) {
                #[cfg(feature = "mcp")]
                tracing::warn!(
                    fact_id = from,
                    target = hit.id,
                    %error,
                    "auto_link edge skipped"
                );
                #[cfg(not(feature = "mcp"))]
                let _ = error;
            }
        }
    }
}
