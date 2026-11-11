//! Anchored remember and recall — the service side of [`crate::anchor`].
//! A `#[path]` child of `service`, `persistence` only, like `reinforce.rs`
//! whose re-rank it extends.

use super::recall::recollection;
use super::{Link, MemoryService, Metadata};
use crate::anchor::{
    anchors_entry, check_anchor_count, payload_freshness, AnchorError, AnchorProbe, AnchorSet,
    AnchoredRecollection, Freshness,
};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::storage::{FactStore, GraphStore, RecallStore};

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// [`Self::remember_with_ttl`], anchoring the lesson to `anchors`: the
    /// files it is about and their content digest now.
    ///
    /// The anchors are stored under a reserved payload key, invisible in
    /// caller metadata. An empty `anchors` stores the fact exactly as
    /// [`Self::remember_with_ttl`] does — and, like it, keeps the anchors an
    /// earlier remember of the same fact recorded; non-empty `anchors`
    /// replace them, which is how a re-verified lesson is re-anchored.
    ///
    /// Build `anchors` with [`crate::anchor::AnchorRoot::anchors`],
    /// so each digest is read by the server from the file itself.
    ///
    /// # Errors
    /// [`MemoryError::Anchor`] for more than
    /// [`crate::anchor::MAX_ANCHORS_PER_FACT`] anchors, checked before
    /// anything is written; otherwise the errors of [`Self::remember_with_ttl`].
    pub fn remember_anchored(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
        anchors: Option<&AnchorSet>,
    ) -> Result<u64, MemoryError>
    where
        S: GraphStore,
    {
        let system = Self::anchor_system(anchors)?;
        let _generation = self.enter_generation();
        self.remember_inner(fact, links, metadata, ttl_seconds, true, system.as_ref())
    }

    /// [`Self::remember_anchored`], additionally returning the `similar`
    /// page — the at-most [`SIMILAR_LIMIT`](super::SIMILAR_LIMIT)
    /// pre-existing memories nearest to the fact, scored by raw similarity,
    /// the fact itself and internal hubs excluded — and, when the service
    /// was built with [`Self::with_auto_link`], a `related_to` edge to each
    /// hit at or above that threshold. [`Self::remember_detailed`] is the
    /// anchor-free twin; both reuse the embedding the store pass already
    /// computed, so one remember costs one embed, never two.
    ///
    /// # Errors
    /// The errors of [`Self::remember_anchored`].
    pub fn remember_anchored_detailed(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
        anchors: Option<&AnchorSet>,
    ) -> Result<crate::model::RememberOutcome, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let system = Self::anchor_system(anchors)?;
        let _generation = self.enter_generation();
        self.remember_outcome(fact, links, metadata, ttl_seconds, system.as_ref())
    }

    /// Validate the requested `anchors` and build the reserved system
    /// entries they contribute — `None` when the set is empty, so storing
    /// stays byte-identical to an unanchored remember.
    fn anchor_system(anchors: Option<&AnchorSet>) -> Result<Option<Metadata>, MemoryError> {
        let files = anchors.map_or(&[][..], |set| set.files.as_slice());
        check_anchor_count(files.len())?;
        if !files.is_empty() && anchors.is_some_and(|set| set.root.is_empty()) {
            return Err(MemoryError::Anchor(AnchorError::MissingRoot));
        }
        Ok(anchors
            .filter(|set| !set.files.is_empty())
            .map(anchors_entry))
    }

    /// [`Self::recall`], with each lesson's [`Freshness`] judged through
    /// `probe`: a lesson whose anchored file changed or disappeared is
    /// [`Freshness::Stale`], and its ranking key is multiplied by
    /// [`crate::anchor::STALE_RANK_FACTOR`] in the same sort that applies
    /// learned confidence. Scores stay the raw similarity.
    ///
    /// Freshness is computed, never stored: it does not touch the learned
    /// confidence, and a lesson whose file is restored is fresh again.
    ///
    /// # Errors
    /// The errors of [`Self::recall`]. A file the probe cannot read is not an
    /// error: that anchor is unverifiable.
    pub fn recall_anchored(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
        probe: &mut dyn AnchorProbe,
    ) -> Result<Vec<AnchoredRecollection>, MemoryError>
    where
        S: RecallStore,
    {
        // Freshness is judged AFTER the generation gate is released: probing
        // a file is I/O, and I/O under the gate parks every generation
        // writer — and, behind a queued writer, every reader of the daemon.
        let (hits, payloads) = {
            let _generation = self.enter_generation();
            let Some(candidates) = self.recall_candidates(query, k, filter)? else {
                return Ok(Vec::new());
            };
            candidates
        };
        let verdicts: Vec<Freshness> = payloads
            .iter()
            .map(|payload| payload_freshness(payload.as_ref(), probe))
            .collect();
        Ok(
            Self::rl_rerank_tagged(hits, payloads, verdicts, Freshness::rank_factor)
                .into_iter()
                .map(|(hit, payload, freshness)| AnchoredRecollection {
                    recollection: recollection((hit, payload)),
                    freshness,
                })
                .collect(),
        )
    }
}

#[cfg(test)]
#[path = "service_anchor_tests.rs"]
mod tests;
