//! Anchored remember and recall — the service side of [`crate::anchor`].
//! A `#[path]` child of `service`, `persistence` only, like `reinforce.rs`
//! whose re-rank it extends.
//!
//! Recall is split in two so no file is read under a lock: the candidate
//! search runs under the generation gate (and, behind the MCP server, under
//! the live-generation slot), and returns owned data; the anchored files are
//! read and judged afterwards, with nothing held. A probe that blocks or
//! takes long then delays one answer, never a generation writer — nor,
//! behind a writer queued on a fair lock, every other reader of the daemon.

use super::recall::{recollection, Hit};
use super::reinforce::rerank_tagged;
use super::{Link, MemoryService, Metadata};
use crate::anchor::{
    anchors_entry, check_anchor_count, payload_freshness, AnchorProbe, AnchorSet,
    AnchoredRecollection, Freshness,
};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::storage::{FactStore, GraphStore, RecallStore};

/// Recall candidates whose anchors are not judged yet: what
/// [`MemoryService::anchored_candidates`] found under the store's locks, for
/// [`Self::judge`] to check against the files once those locks are released.
#[derive(Debug, Default)]
#[must_use = "candidates are judged by `judge`, which reads the anchored files"]
pub struct AnchoredCandidates {
    hits: Vec<Hit>,
    payloads: Vec<Option<Metadata>>,
}

impl AnchoredCandidates {
    /// Judge each candidate's [`Freshness`] through `probe` and rank them: a
    /// stale lesson's ranking key is multiplied by
    /// [`crate::anchor::STALE_RANK_FACTOR`] in the same sort that applies
    /// learned confidence. Scores stay the raw similarity. Holds no lock: call
    /// it after the call that produced the candidates has returned.
    #[must_use]
    pub fn judge(self, probe: &mut dyn AnchorProbe) -> Vec<AnchoredRecollection> {
        let verdicts: Vec<Freshness> = self
            .payloads
            .iter()
            .map(|payload| payload_freshness(payload.as_ref(), probe))
            .collect();
        rerank_tagged(self.hits, self.payloads, verdicts, Freshness::rank_factor)
            .into_iter()
            .map(|(hit, payload, freshness)| AnchoredRecollection {
                recollection: recollection((hit, payload)),
                freshness,
            })
            .collect()
    }
}

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// [`Self::remember_with_ttl`], anchoring the lesson to `anchors`: the
    /// files it is about, their content digest now, and the checkout root
    /// they were read under.
    ///
    /// The anchors are stored under a reserved payload key, invisible in
    /// caller metadata. `None` (or a set with no file) stores the fact exactly
    /// as [`Self::remember_with_ttl`] does — and, like it, keeps the anchors
    /// an earlier remember of the same fact recorded; a non-empty set
    /// replaces them, which is how a re-verified lesson is re-anchored.
    ///
    /// Build the set with [`crate::anchor::AnchorRoot::anchors`], so each
    /// digest is read by the server from the file itself.
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
        let anchors = anchors.filter(|set| !set.files.is_empty());
        if let Some(set) = anchors {
            check_anchor_count(set.files.len())?;
        }
        let system = anchors.map(anchors_entry);
        let _generation = self.enter_generation();
        self.remember_inner(fact, links, metadata, ttl_seconds, true, system.as_ref())
    }

    /// The candidates [`Self::recall`] would rank, with their raw payloads,
    /// found under the generation gate and returned before any anchored
    /// file is read: pass them to [`AnchoredCandidates::judge`].
    ///
    /// # Errors
    /// The errors of [`Self::recall`].
    pub fn anchored_candidates(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
    ) -> Result<AnchoredCandidates, MemoryError>
    where
        S: RecallStore,
    {
        let _generation = self.enter_generation();
        Ok(self
            .recall_candidates(query, k, filter)?
            .map(|(hits, payloads)| AnchoredCandidates { hits, payloads })
            .unwrap_or_default())
    }

    /// [`Self::recall`], with each lesson's [`Freshness`] judged through
    /// `probe`: [`Self::anchored_candidates`], then
    /// [`AnchoredCandidates::judge`] once the gate is released. A lesson
    /// whose anchored file changed or disappeared is [`Freshness::Stale`] and
    /// ranked lower; one anchored under another checkout root than the
    /// probe's is [`Freshness::Unverified`] and keeps its rank.
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
        Ok(self.anchored_candidates(query, k, filter)?.judge(probe))
    }
}

#[cfg(test)]
#[path = "service_anchor_tests.rs"]
mod tests;
