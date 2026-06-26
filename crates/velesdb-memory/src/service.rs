//! The memory service: five operations over the in-core Agent Memory SDK.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use velesdb_core::agent::AgentMemory;
use velesdb_core::Database;

/// Structured metadata attached to a memory (the `ColumnStore` facet): exact-match
/// fields like `project`, `author`, `type`, `status`, `date`. `content` and
/// `_veles_expires_at` are reserved keys.
pub type Metadata = Map<String, Value>;

use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::id;

/// A typed link from a freshly remembered fact to an existing memory.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct Link {
    /// Id of the memory being linked to.
    pub target: u64,
    /// Relationship label (e.g. `"decided_in"`, `"references"`, `"depends_on"`).
    pub relation: String,
}

/// One semantically recalled memory.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Recollection {
    /// Stable id of the memory.
    pub id: u64,
    /// Similarity score (higher is closer).
    pub score: f32,
    /// Stored fact content.
    pub content: String,
}

/// A node in an [`Explanation`] subgraph.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemoryNode {
    /// Stable id of the memory.
    pub id: u64,
    /// Stored fact content.
    pub content: String,
    /// Distance in hops from the seed memory (the seed is hop `0`).
    pub hop: usize,
}

/// A typed edge in an [`Explanation`] subgraph.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MemoryEdge {
    /// Source memory id.
    pub from: u64,
    /// Target memory id.
    pub to: u64,
    /// Relationship label.
    pub relation: String,
}

/// The connected answer to a `why` question: the best-matching seed memory plus
/// everything reachable from it within a hop budget. This connected subgraph is
/// the differentiator — it surfaces related memories a purely vector recall is
/// blind to (no textual similarity required).
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct Explanation {
    /// Memories in the subgraph, seed first.
    pub nodes: Vec<MemoryNode>,
    /// Typed edges connecting the nodes.
    pub edges: Vec<MemoryEdge>,
}

/// Local-first agent memory backed by a single `VelesDB` instance.
///
/// Generic over the [`Embedder`] so production can use an on-device model while
/// tests use a deterministic, network-free one.
pub struct MemoryService<E: Embedder> {
    memory: AgentMemory,
    embedder: E,
}

impl<E: Embedder> MemoryService<E> {
    /// Open (or create) a memory store at `path`, using `embedder` for text
    /// vectorization. The store never leaves this directory.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the store cannot be opened or the agent
    /// memory cannot be initialized for the embedder's dimension.
    pub fn open<P: AsRef<Path>>(path: P, embedder: E) -> Result<Self, MemoryError> {
        let db = Arc::new(Database::open(path)?);
        let memory = AgentMemory::with_dimension(db, embedder.dimension())?;
        Ok(Self { memory, embedder })
    }

    /// Remember a `fact`, optionally tagging it with structured `metadata`
    /// (`ColumnStore` facet) and linking it to existing memories (graph facet).
    /// Returns the stable id of the fact (idempotent on identical content).
    ///
    /// Link targets are validated to exist *before* the fact is stored, so a bad
    /// link never leaves the fact half-written.
    ///
    /// # Errors
    /// Returns [`MemoryError::EmptyFact`] for empty/whitespace facts,
    /// [`MemoryError::UnknownLinkTarget`] if a link points at a missing memory,
    /// or a storage error if persistence fails.
    pub fn remember(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
    ) -> Result<u64, MemoryError> {
        if fact.trim().is_empty() {
            return Err(MemoryError::EmptyFact);
        }
        self.ensure_link_targets_exist(links)?;
        let fact_id = id::stable_id(fact);
        let embedding = self.embedder.embed(fact)?;
        self.store(fact_id, fact, &embedding, metadata)?;
        for link in links {
            self.memory
                .semantic()
                .relate(fact_id, link.target, &link.relation, None)?;
        }
        Ok(fact_id)
    }

    /// Fail unless every link target already exists (keeps `remember` atomic).
    fn ensure_link_targets_exist(&self, links: &[Link]) -> Result<(), MemoryError> {
        for link in links {
            if self.memory.semantic().get(link.target)?.is_none() {
                return Err(MemoryError::UnknownLinkTarget(link.target));
            }
        }
        Ok(())
    }

    /// Store a fact with or without metadata.
    fn store(
        &self,
        id: u64,
        fact: &str,
        embedding: &[f32],
        metadata: Option<&Metadata>,
    ) -> Result<(), MemoryError> {
        match metadata {
            Some(meta) => self
                .memory
                .semantic()
                .store_with_metadata(id, fact, embedding, meta)
                .map_err(MemoryError::from),
            None => self
                .memory
                .semantic()
                .store(id, fact, embedding)
                .map_err(MemoryError::from),
        }
    }

    /// Recall up to `k` memories semantically similar to `query` (vector facet),
    /// optionally narrowed to an exact-match metadata `filter` (`ColumnStore`
    /// facet) — e.g. `{ "project": "veles", "status": "resolved" }`.
    ///
    /// The filter is applied *after* vector ranking, so a highly selective
    /// filter over a large, dissimilar corpus may return fewer than `k` hits
    /// even when more matches exist further down the ranking — raise `k` if you
    /// need exhaustive filtered recall.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the semantic query fails.
    pub fn recall(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
    ) -> Result<Vec<Recollection>, MemoryError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let embedding = self.embedder.embed(query)?;
        let hits = self.search(&embedding, k, filter)?;
        Ok(hits
            .into_iter()
            .map(|(id, score, content)| Recollection { id, score, content })
            .collect())
    }

    /// Vector search for up to `k` ids, optionally narrowed by a metadata
    /// `filter`. Shared by [`Self::recall`] and [`Self::why`].
    fn search(
        &self,
        embedding: &[f32],
        k: usize,
        filter: Option<&Metadata>,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        match filter {
            Some(meta) => self
                .memory
                .semantic()
                .query_filtered(embedding, k, meta, 0)
                .map_err(MemoryError::from),
            None => self
                .memory
                .semantic()
                .query(embedding, k)
                .map_err(MemoryError::from),
        }
    }

    /// Create a typed edge `from -> to`. Returns the edge id.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the edge cannot be created.
    pub fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
        self.memory
            .semantic()
            .relate(from, to, relation, None)
            .map_err(MemoryError::from)
    }

    /// Forget (delete) the memory with `fact_id`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the deletion fails.
    pub fn forget(&self, fact_id: u64) -> Result<(), MemoryError> {
        self.memory
            .semantic()
            .delete(fact_id)
            .map_err(MemoryError::from)
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
    ) -> Result<Explanation, MemoryError> {
        if decision.trim().is_empty() {
            return Ok(Explanation::default());
        }
        let embedding = self.embedder.embed(decision)?;
        let seeds = self.search(&embedding, 1, filter)?;
        let Some((seed_id, _score, seed_content)) = seeds.into_iter().next() else {
            return Ok(Explanation::default());
        };
        self.traverse(seed_id, seed_content, max_hops)
    }

    /// Breadth-first walk over outgoing links from `seed_id`, collecting nodes
    /// and edges up to `max_hops` away.
    fn traverse(
        &self,
        seed_id: u64,
        seed_content: String,
        max_hops: usize,
    ) -> Result<Explanation, MemoryError> {
        let mut explanation = Explanation {
            nodes: vec![MemoryNode {
                id: seed_id,
                content: seed_content,
                hop: 0,
            }],
            edges: Vec::new(),
        };
        let mut visited: HashSet<u64> = HashSet::from([seed_id]);
        let mut frontier = vec![seed_id];
        for hop in 1..=max_hops {
            let mut next = Vec::new();
            for node_id in frontier.drain(..) {
                self.expand(node_id, hop, &mut explanation, &mut visited, &mut next)?;
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        Ok(explanation)
    }

    /// Expand a single node: enqueue unseen targets and record edges. An edge is
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
    ) -> Result<(), MemoryError> {
        for edge in self.memory.semantic().relations(node_id)? {
            let target = edge.target();
            if !visited.contains(&target) {
                let Some((content, _embedding)) = self.memory.semantic().get(target)? else {
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
            explanation.edges.push(MemoryEdge {
                from: edge.source(),
                to: target,
                relation: edge.label().to_owned(),
            });
        }
        Ok(())
    }
}
