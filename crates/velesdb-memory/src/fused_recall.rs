//! [`MemoryService::recall_fused`]: vector recall combined with the graph
//! reach `why()` already walks, re-ranked by [`crate::fusion::fuse`]. Split
//! out of `service.rs` to keep that file under the crate's NLOC budget; a
//! child module of `service`, so it freely uses `MemoryService`'s private
//! fields and methods (`traverse`, `search`, `HUB_FIELD`, …).

use serde_json::Value;

use super::{reject_reserved_keys, MemoryService, Metadata, HUB_FIELD, MENTIONS_RELATION};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::fusion::{self, Candidate};
use crate::model::{FusionOptions, MemoryEdge, MemoryNode, Recollection};
use crate::rerank::Reranker;

impl<E: Embedder> MemoryService<E> {
    /// Fused recall: like [`Self::recall`], but also walks the graph from the
    /// query's top vector hit and folds any fact it reaches (hop ≥ 1) into the
    /// ranking, scored by `opts.graph_boost · graph_weight` on top of its
    /// normalised vector similarity. A fact the graph reaches never displaces
    /// a strong vector hit unless the boosted score genuinely outranks it; a
    /// fact the vector pool ranked low (or missed) can still surface if the
    /// graph connects it. This is the tri-engine ranking measured on
    /// HotpotQA/TimeQA/LoCoMo (`examples/multihop`, `examples/timeqa`,
    /// `examples/locomo`) — [`Self::recall`] stays pure-vector and unchanged,
    /// so existing callers see no behavior shift.
    ///
    /// The graph reach requires a wired graph to find anything: it walks
    /// edges from [`Self::relate`] or the entity hubs
    /// [`Self::remember_extracted`] auto-wires. Entity hubs themselves are
    /// never returned, exactly like [`Self::recall`].
    ///
    /// # Errors
    /// Returns [`MemoryError`] if embedding, vector search, or graph
    /// traversal fails.
    pub fn recall_fused(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
        opts: FusionOptions,
    ) -> Result<Vec<Recollection>, MemoryError> {
        let query = query.trim();
        if query.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        reject_reserved_keys(filter)?;
        let embedding = self.embedder.embed(query)?;
        let pool = self.fused_pool(&embedding, pool_depth(k, opts), filter)?;
        let reached = self.graph_reached(&embedding, filter, opts.hops)?;
        Ok(fusion::fuse(pool, &reached, k, opts.graph_boost))
    }

    /// Like [`Self::recall_fused`], but hands the FULL fused-ranked candidate
    /// pool (before the final `k` cutoff) to `reranker` for a second-stage
    /// re-score, then truncates to `k`. Closes the ranking-miss gap the
    /// `LoCoMo` ceiling diagnostic found: a relevant fact can be IN the pool
    /// (recall@64 ≈ 89% on multi-hop) yet outranked out of a tight `k`
    /// (recall@8 ≈ 50%) — a reranker recovers it without widening `k` itself.
    ///
    /// No built-in reranker ships: bring your own (cross-encoder, LLM judge,
    /// …) via [`Reranker`]. Never call this as a default — a reranker can
    /// also *hurt* out-of-distribution conversational queries (measured on
    /// `LoCoMo`), so it is opt-in, one call at a time.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if embedding, vector search, graph traversal,
    /// or `reranker` itself fails.
    pub fn recall_fused_reranked<R: Reranker>(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
        opts: FusionOptions,
        reranker: &R,
    ) -> Result<Vec<Recollection>, MemoryError> {
        let query = query.trim();
        if query.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        reject_reserved_keys(filter)?;
        let embedding = self.embedder.embed(query)?;
        let depth = pool_depth(k, opts);
        let pool = self.fused_pool(&embedding, depth, filter)?;
        let reached = self.graph_reached(&embedding, filter, opts.hops)?;
        let fused = fusion::fuse(pool, &reached, depth, opts.graph_boost);
        let ranked = reranker.rerank(query, fused)?;
        Ok(ranked.into_iter().take(k).collect())
    }

    /// The oversampled vector pool [`Self::recall_fused`] re-ranks.
    fn fused_pool(
        &self,
        embedding: &[f32],
        depth: usize,
        filter: Option<&Metadata>,
    ) -> Result<Vec<Candidate>, MemoryError> {
        let hits = self.search(embedding, depth, filter)?;
        hits.into_iter()
            .map(|(id, score, content)| {
                Ok(Candidate {
                    recollection: Recollection {
                        id,
                        score,
                        content,
                        metadata: self.recall_metadata(id)?,
                    },
                    vector_score: f64::from(score),
                    graph_weight: 0.0,
                })
            })
            .collect()
    }

    /// Facts the graph reaches (hop ≥ 1) from the query's top vector seed,
    /// entity hubs excluded, each weighted by [`Self::reach_weight`]: a link
    /// through a rare, specific entity hub promotes harder than one through a
    /// generic mega-hub whose connections carry little signal — the idf lever
    /// validated on `HotpotQA` (+5.0pp both-facts-complete) and `LoCoMo` (turns
    /// the graph net-positive on multi-hop, no regression elsewhere).
    fn graph_reached(
        &self,
        embedding: &[f32],
        filter: Option<&Metadata>,
        hops: usize,
    ) -> Result<Vec<Candidate>, MemoryError> {
        let seeds = self.search(embedding, 1, filter)?;
        let Some((seed_id, _score, seed_content)) = seeds.into_iter().next() else {
            return Ok(Vec::new());
        };
        let explanation = self.traverse(seed_id, seed_content, hops)?;
        let mut reached = Vec::new();
        for node in &explanation.nodes {
            if let Some(candidate) = self.reached_candidate(node, &explanation.edges)? {
                reached.push(candidate);
            }
        }
        Ok(reached)
    }

    /// The graph-reached candidate for `node`, or `None` when it's the seed
    /// itself (hop 0) or an entity hub (internal scaffolding, never a caller
    /// fact — split out of [`Self::graph_reached`] to keep that loop's
    /// complexity within budget).
    fn reached_candidate(
        &self,
        node: &MemoryNode,
        edges: &[MemoryEdge],
    ) -> Result<Option<Candidate>, MemoryError> {
        if node.hop == 0 || self.is_hub(node.id)? {
            return Ok(None);
        }
        let weight = self.reach_weight(node.id, edges)?;
        Ok(Some(Candidate {
            recollection: Recollection {
                id: node.id,
                score: 0.0,
                content: node.content.clone(),
                metadata: self.recall_metadata(node.id)?,
            },
            vector_score: 0.0,
            graph_weight: weight,
        }))
    }

    /// The strength of the link(s) that reached `fact_id`: the maximum
    /// entity-idf ([`Self::entity_idf`]) over every hub with a `mentions`
    /// edge into it, or a flat `1.0` when it was reached through a direct
    /// (non-hub) [`Self::relate`] edge instead — idf has nothing to weight
    /// there, so the original flat signal is kept.
    fn reach_weight(&self, fact_id: u64, edges: &[MemoryEdge]) -> Result<f64, MemoryError> {
        let mut weight: Option<f64> = None;
        for edge in edges {
            if edge.to == fact_id && edge.relation == MENTIONS_RELATION {
                let idf = self.entity_idf(edge.from)?;
                weight = Some(weight.map_or(idf, |w: f64| w.max(idf)));
            }
        }
        Ok(weight.unwrap_or(1.0))
    }

    /// Normalised inverse document frequency of hub `hub_id`, in `[0, 1]`:
    /// `1` when it links a single fact (maximally specific), trending to `0`
    /// as it links ever more (a generic mega-hub whose links carry little
    /// answer signal). Mirrors the `LoCoMo` harness formula
    /// (`examples/locomo/ingest.rs`), using the store's total memory count
    /// (facts + hubs) as a corpus-size proxy.
    fn entity_idf(&self, hub_id: u64) -> Result<f64, MemoryError> {
        let degree = self.memory.semantic().relations(hub_id)?.len();
        let n = self.memory.semantic().count();
        if degree == 0 || n <= 1 {
            return Ok(0.0);
        }
        #[allow(clippy::cast_precision_loss)] // corpus/degree sizes are far below f64's exact range
        let (n, d) = (n as f64, degree as f64);
        Ok((n / d).ln() / n.ln())
    }

    /// True when `id` is an entity hub auto-created by
    /// [`Self::remember_extracted`] (internal scaffolding, never a caller
    /// fact — recall and graph reach both exclude it).
    fn is_hub(&self, id: u64) -> Result<bool, MemoryError> {
        Ok(self
            .memory
            .semantic()
            .get_metadata(id)?
            .is_some_and(|meta| meta.get(HUB_FIELD) == Some(&Value::Bool(true))))
    }
}

/// The oversampled candidate pool depth for a `k`-sized fused recall:
/// `opts.pool` if the caller set one, else the proven default
/// ([`fusion::pool_size`]).
fn pool_depth(k: usize, opts: FusionOptions) -> usize {
    opts.pool.unwrap_or_else(|| fusion::pool_size(k))
}
