//! The graph facet of [`MemoryService`]: `relate`/`unrelate`/`forget`, the
//! entity-hub lifecycle, and the `why`/`traverse`/`expand` walks — split out
//! to keep `service.rs` inside the crate's file budget, same pattern as
//! `fused_recall.rs`. A child module of `service`, so it shares full access
//! to `MemoryService`'s private fields and methods. Every method here needs
//! at least `S: GraphStore` (#1959) — that is the seam this file cuts along.

use super::*;

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// Create a typed edge `from -> to`. Returns the edge id.
    ///
    /// Both endpoints are validated to exist first, so the tool reports an
    /// unknown id as client input (`UnknownMemory`) rather than a generic
    /// storage fault — and the graph never gains an edge dangling off a memory
    /// that was never stored.
    ///
    /// A self-loop (`from == to`) is refused: it states nothing, and `why`
    /// traverses it like any other edge, so it only adds noise to the
    /// evidence trail. The same rule covers [`Self::remember`]'s `links`.
    ///
    /// # Errors
    /// Returns [`MemoryError::InvalidRelation`] for a bad label,
    /// [`MemoryError::SelfRelation`] if both endpoints are the same memory,
    /// [`MemoryError::UnknownMemory`] if either endpoint is missing, or
    /// a storage error if the edge cannot be created.
    pub fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        self.relate_inner(from, to, relation)
    }

    pub(super) fn relate_inner(
        &self,
        from: u64,
        to: u64,
        relation: &str,
    ) -> Result<u64, MemoryError>
    where
        S: GraphStore,
    {
        validate_relation(relation)?;
        if from == to {
            return Err(MemoryError::SelfRelation(from));
        }
        self.ensure_exists(from)?;
        self.ensure_exists(to)?;
        self.store.relate(from, to, relation)
    }

    /// Remove the edge(s) `from -relation-> to`: [`Self::relate`]'s exact
    /// undo (issue #1661), so a mistaken edge no longer costs the facts at
    /// its endpoints. Neither the facts nor any entity hub are touched —
    /// collecting an orphaned hub stays [`Self::forget`]'s job.
    ///
    /// Idempotent: an absent edge is `found: false`, not an error, so a
    /// cleanup is replayable. It refuses exactly what `relate` refuses
    /// (empty label, self-loop), and deliberately does NOT require the
    /// endpoints to exist — the edge of a forgotten fact is already gone,
    /// and reporting that as an error would break replay.
    ///
    /// Scope: the store does not distinguish an explicit edge from one the
    /// autograph derived from a passage, so `unrelate` removes both alike.
    /// To correct an autograph edge, prefer `forget` + `remember` of the
    /// source fact — otherwise a later `remember` of the same passage can
    /// rebuild the edge removed here.
    ///
    /// # Errors
    /// Returns [`MemoryError::InvalidRelation`] for a bad label,
    /// [`MemoryError::SelfRelation`] if both endpoints are the same memory,
    /// or a storage error if lookup or removal fails.
    pub fn unrelate(
        &self,
        from: u64,
        to: u64,
        relation: &str,
    ) -> Result<UnrelateOutcome, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        validate_relation(relation)?;
        if from == to {
            return Err(MemoryError::SelfRelation(from));
        }
        let removed = self.remove_matching_edges(from, to, relation)?;
        Ok(UnrelateOutcome {
            found: removed > 0,
            removed,
        })
    }

    /// [`Self::unrelate`]'s removal pass: resolve `from`'s outgoing edges and
    /// delete every one matching `(to, relation)` by its id, counting them.
    fn remove_matching_edges(
        &self,
        from: u64,
        to: u64,
        relation: &str,
    ) -> Result<usize, MemoryError>
    where
        S: GraphStore,
    {
        let mut removed = 0usize;
        for edge in self.store.relations(from)? {
            if edge.to == to
                && edge.relation == relation
                && self.store.unrelate_from(from, edge.id)?
            {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Forget (delete) the memory with `fact_id`. Returns whether a memory
    /// actually existed under that id — the underlying store's `delete` is a
    /// silent no-op on an unknown id (matching most backends' idempotent
    /// delete semantics), which is indistinguishable from a real deletion
    /// unless existence is checked first. Every surface that exposes
    /// `forget` (MCP, Node, WASM, Python) forwards this so a caller can tell
    /// "I removed something" from "that id was a typo".
    ///
    /// The delete always runs, even when `get` reports the id absent: `get`
    /// filters TTL-expired facts, and an expired-but-unpurged row must still
    /// be reclaimed (the caller is told `false` — the memory was already
    /// gone from its perspective). Existence check and delete are two store
    /// calls, not one atomic operation: two concurrent forgets of one id may
    /// both report `true`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the existence check or the deletion fails.
    pub fn forget(&self, fact_id: u64) -> Result<bool, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        let found = self.store.get(fact_id)?.is_some();
        // Read the fact's hubs BEFORE the delete: afterwards its edges are gone
        // and there is no way back to the entities it created.
        let hubs = self.hubs_linked_from(fact_id)?;
        self.store.delete(fact_id)?;
        self.collect_orphan_hubs(&hubs)?;
        Ok(found)
    }

    /// The entity hubs `fact_id` points at.
    ///
    /// Hubs are recognised by the reserved [`HUB_FIELD`] marker rather than by
    /// the edge label, so a caller's own `relate` to a hub is seen too.
    fn hubs_linked_from(&self, fact_id: u64) -> Result<Vec<u64>, MemoryError>
    where
        S: GraphStore,
    {
        let mut hubs = Vec::new();
        for edge in self.store.relations(fact_id)? {
            if self.is_hub(edge.to)? {
                hubs.push(edge.to);
            }
        }
        Ok(hubs)
    }

    /// Delete every hub in `hubs` that no surviving fact mentions any more.
    ///
    /// An entity outlives the fact that introduced it as long as another fact
    /// still refers to it — forgetting "Theo is 15" must not erase Theo while
    /// "Theo has a sister" is still stored. Only a hub whose every `mentions`
    /// target is gone is itself removed, so entities do not accumulate as
    /// unreachable scaffolding once the facts behind them are retracted.
    fn collect_orphan_hubs(&self, hubs: &[u64]) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
        for &hub in hubs {
            if !self.hub_still_mentioned(hub)? {
                self.store.delete(hub)?;
            }
        }
        Ok(())
    }

    /// Whether anything alive still needs `hub`.
    ///
    /// Two references count, and the second is why this reads BOTH
    /// directions (issue #1662):
    ///
    /// - an outgoing `mentions` edge to a live fact — the pair
    ///   [`Self::wire_entities`] writes, the ordinary case;
    /// - an incoming edge from a live NON-HUB fact — what a caller's own
    ///   `relate` writes, and it writes one direction only. Relating a fact
    ///   to a hub is reachable (`entity()` hands out the hub id), so reading
    ///   outgoing edges alone swept hubs from under live callers' edges,
    ///   losing them in silence.
    ///
    /// Incoming edges from another HUB are deliberately ignored: hub↔hub
    /// edges exist (`wire_relations` writes them), and counting them would
    /// let two hubs keep each other alive forever — a leak whose outcome
    /// depends on collection order, which is worse than the bug being fixed.
    fn hub_still_mentioned(&self, hub: u64) -> Result<bool, MemoryError>
    where
        S: GraphStore,
    {
        for edge in self.store.relations(hub)? {
            if edge.relation == MENTIONS_RELATION && self.store.get(edge.to)?.is_some() {
                return Ok(true);
            }
        }
        self.hub_has_live_referent(hub)
    }

    /// Whether a live non-hub fact points AT `hub` — see
    /// [`Self::hub_still_mentioned`] for why hub→hub edges do not count.
    fn hub_has_live_referent(&self, hub: u64) -> Result<bool, MemoryError>
    where
        S: GraphStore,
    {
        for edge in self.store.incoming_relations(hub)? {
            if self.store.get(edge.from)?.is_some() && !self.is_hub(edge.from)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether `id` is an entity hub (carries the reserved [`HUB_FIELD`]).
    fn is_hub(&self, id: u64) -> Result<bool, MemoryError> {
        Ok(self
            .store
            .get_metadata(id)?
            .is_some_and(|meta| meta.contains_key(HUB_FIELD)))
    }

    /// Explain a `decision`: find the best-matching memory (optionally scoped to
    /// a metadata `filter`, e.g. the current project), then walk its typed links
    /// up to `max_hops` away — fusing the vector, `ColumnStore`, and graph facets.
    ///
    /// Returns an empty [`Explanation`] when nothing matches the decision.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if recall or graph traversal fails.
    pub fn why(
        &self,
        decision: &str,
        max_hops: usize,
        filter: Option<&Metadata>,
    ) -> Result<Explanation, MemoryError>
    where
        S: GraphStore + RecallStore,
    {
        let _generation = self.enter_generation();
        let decision = decision.trim();
        if decision.is_empty() {
            return Ok(Explanation::default());
        }
        reject_reserved_keys(filter)?;
        let embedding = self.embedder.embed(decision)?;
        let seeds = self.search(&embedding, 1, filter)?;
        let Some((seed_id, _score, seed_content)) = seeds.into_iter().next() else {
            return Ok(Explanation::default());
        };
        self.traverse(seed_id, seed_content, max_hops)
    }

    /// Breadth-first walk over outgoing links from `seed_id`, collecting nodes
    /// and edges up to `max_hops` away.
    pub(super) fn traverse(
        &self,
        seed_id: u64,
        seed_content: String,
        max_hops: usize,
    ) -> Result<Explanation, MemoryError>
    where
        S: GraphStore,
    {
        let mut explanation = Explanation {
            nodes: vec![MemoryNode {
                id: seed_id,
                content: seed_content,
                hop: 0,
            }],
            edges: Vec::new(),
            truncated: false,
        };
        let mut visited: HashSet<u64> = HashSet::from([seed_id]);
        let mut frontier = vec![seed_id];
        let mut next: Vec<u64> = Vec::new();
        'hops: for hop in 1..=max_hops {
            next.clear();
            for node_id in frontier.drain(..) {
                // Both width budgets, checked here AND inside `expand`: this
                // check alone would let the expansion that crosses the line
                // finish its node — up to MAX_WHY_NODE_DEGREE nodes past the
                // "ceiling", which a review measured at 522 of a promised 500.
                if explanation.nodes.len() >= crate::limits::MAX_WHY_NODES
                    || explanation.edges.len() >= crate::limits::MAX_WHY_EDGES
                {
                    // Unexpanded frontier work remained — the response is a
                    // partial view and must SAY so (#1820); whether the rest
                    // held anything unseen is exactly what the budget forbids
                    // finding out, so the cautious true is the honest one.
                    explanation.truncated = true;
                    break 'hops; // width budget spent — depth left in max_hops is moot
                }
                self.expand(node_id, hop, &mut explanation, &mut visited, &mut next)?;
            }
            if next.is_empty() {
                break;
            }
            std::mem::swap(&mut frontier, &mut next);
        }
        Ok(explanation)
    }

    /// Expand a single node: enqueue unseen targets and record edges, following
    /// at most [`crate::limits::MAX_WHY_NODE_DEGREE`] outgoing edges — an entity
    /// hub's degree scales with the whole store, so an unbounded walk here would
    /// dump its entire neighborhood into one response (issue #1743). An edge is
    /// only recorded once its target is a resolved node, so the subgraph never
    /// contains an edge pointing at a node absent from `nodes` (e.g. a forgotten
    /// target whose edge outlived it).
    fn expand(
        &self,
        node_id: u64,
        hop: usize,
        explanation: &mut Explanation,
        visited: &mut HashSet<u64>,
        next: &mut Vec<u64>,
    ) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
        // The bounded read pushes the per-node budget into the store's own
        // index scan: the old full fetch materialized a super-node's whole
        // degree before `.take()` could apply — O(store size) transient
        // allocation at a single hop, the cost half of #1743 that #1820
        // closes. The store also reports whether the degree exceeded the
        // budget, which is what makes the cut OBSERVABLE.
        let bounded = self
            .store
            .relations_bounded(node_id, crate::limits::MAX_WHY_NODE_DEGREE)?;
        if bounded.truncated {
            explanation.truncated = true;
        }
        for edge in bounded.edges {
            // The budgets are ceilings, not suggestions: once either is spent,
            // this node's expansion stops MID-NODE rather than finishing. The
            // caller's check between nodes cannot provide that — an expansion
            // that crosses the line would otherwise add its whole degree.
            if explanation.nodes.len() >= crate::limits::MAX_WHY_NODES
                || explanation.edges.len() >= crate::limits::MAX_WHY_EDGES
            {
                // An edge was in hand and not followed — an exact cut, not
                // a conservative one.
                explanation.truncated = true;
                break;
            }
            let target = edge.to;
            if !visited.contains(&target) {
                let Some((content, _embedding)) = self.store.get(target)? else {
                    continue; // target no longer exists → drop the dangling edge too
                };
                visited.insert(target);
                explanation.nodes.push(MemoryNode {
                    id: target,
                    content,
                    hop,
                });
                next.push(target);
            }
            explanation.edges.push(edge);
        }
        Ok(())
    }
}
