//! The memory service: five operations over the in-core Agent Memory SDK.

use std::collections::{HashMap, HashSet};
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
use crate::extract::Extractor;
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
    /// [`MemoryError::UnknownMemory`] if a link points at a missing memory,
    /// or a storage error if persistence fails.
    pub fn remember(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
    ) -> Result<u64, MemoryError> {
        let fact = fact.trim();
        if fact.is_empty() {
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

    /// Remember a passage of raw `text` by running it through an [`Extractor`]
    /// and storing every fact it yields, **auto-wiring the fact↔entity graph**.
    ///
    /// This is the commodity on top of [`Self::remember`]'s bring-your-own-links
    /// core: each extracted fact is stored (tagged with `metadata`), each salient
    /// topic becomes a deduplicated hub memory, and every fact is linked to its
    /// topics with a bidirectional `about`/`mentions` edge. Two facts sharing a
    /// topic therefore become reachable from one another, so [`Self::why`] has a
    /// real graph to traverse with no manual `relate()`.
    ///
    /// Entity hubs are content-addressed, so the same topic seen across many
    /// calls collapses onto one hub. Returns the ids of the stored facts (entity
    /// hubs excluded), in extraction order.
    ///
    /// # Errors
    /// Returns [`MemoryError::EmptyFact`] for empty/whitespace `text`,
    /// [`MemoryError::Extract`] if extraction fails, or a storage error if
    /// persistence fails.
    pub fn remember_extracted<X: Extractor>(
        &self,
        text: &str,
        extractor: &X,
        metadata: Option<&Metadata>,
    ) -> Result<Vec<u64>, MemoryError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MemoryError::EmptyFact);
        }
        let facts = extractor.extract(text)?;
        let mut fact_ids = Vec::new();
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64)> = HashSet::new();
        for fact in &facts {
            let content = fact.text.trim();
            if content.is_empty() {
                continue;
            }
            let fact_id = self.remember(content, &[], metadata)?;
            fact_ids.push(fact_id);
            self.wire_entities(fact_id, &fact.entities, &mut entity_ids, &mut edges)?;
        }
        Ok(fact_ids)
    }

    /// Link `fact_id` to each of its topics with a deduplicated edge in *both*
    /// directions. `why()` only follows outgoing edges, so the fact→topic edge
    /// alone leaves hubs as dead ends; the topic→fact edge is what lets a walk
    /// hop from one fact, through a shared topic, to its sibling facts.
    fn wire_entities(
        &self,
        fact_id: u64,
        entities: &[String],
        entity_ids: &mut HashMap<String, u64>,
        edges: &mut HashSet<(u64, u64)>,
    ) -> Result<(), MemoryError> {
        for entity in entities {
            // Skip blank or punctuation-only topics: they would persist as junk
            // hubs (`Entity: -`) yet can never carry a meaningful multi-hop link.
            if entity.chars().any(char::is_alphanumeric) {
                self.wire_entity(fact_id, entity, entity_ids, edges)?;
            }
        }
        Ok(())
    }

    /// Wire one topic to `fact_id`: resolve its hub, then add the deduplicated
    /// `about`/`mentions` pair (skipping a hub that is the fact itself).
    fn wire_entity(
        &self,
        fact_id: u64,
        entity: &str,
        entity_ids: &mut HashMap<String, u64>,
        edges: &mut HashSet<(u64, u64)>,
    ) -> Result<(), MemoryError> {
        let entity_id = self.entity_hub(entity, entity_ids)?;
        if entity_id == fact_id {
            return Ok(());
        }
        if edges.insert((fact_id, entity_id)) {
            self.relate(fact_id, entity_id, "about")?;
        }
        if edges.insert((entity_id, fact_id)) {
            self.relate(entity_id, fact_id, "mentions")?;
        }
        Ok(())
    }

    /// Get or create the hub memory for a topic, caching its id per call. The
    /// hub content is canonical (`Entity: <topic>`), so its stable id is the same
    /// across calls and the same topic never spawns a duplicate hub.
    fn entity_hub(
        &self,
        entity: &str,
        entity_ids: &mut HashMap<String, u64>,
    ) -> Result<u64, MemoryError> {
        let key = entity.trim().to_lowercase();
        if let Some(&id) = entity_ids.get(&key) {
            return Ok(id);
        }
        let mut meta = Map::new();
        meta.insert("kind".to_string(), Value::String("entity".to_string()));
        let id = self.remember(&format!("Entity: {key}"), &[], Some(&meta))?;
        entity_ids.insert(key, id);
        Ok(id)
    }

    /// Fail with [`MemoryError::UnknownMemory`] unless memory `id` exists.
    fn ensure_exists(&self, id: u64) -> Result<(), MemoryError> {
        if self.memory.semantic().get(id)?.is_none() {
            return Err(MemoryError::UnknownMemory(id));
        }
        Ok(())
    }

    /// Fail unless every link target already exists (keeps `remember` atomic).
    fn ensure_link_targets_exist(&self, links: &[Link]) -> Result<(), MemoryError> {
        for link in links {
            self.ensure_exists(link.target)?;
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
    /// A highly selective filter may return fewer than `k` hits even when more
    /// matches exist — raise `k` for fuller coverage with a narrow filter.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the semantic query fails.
    pub fn recall(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
    ) -> Result<Vec<Recollection>, MemoryError> {
        let query = query.trim();
        if query.is_empty() {
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
    /// Both endpoints are validated to exist first, so the tool reports an
    /// unknown id as client input (`UnknownMemory`) rather than a generic
    /// storage fault — and the graph never gains an edge dangling off a memory
    /// that was never stored.
    ///
    /// # Errors
    /// Returns [`MemoryError::UnknownMemory`] if either endpoint is missing, or
    /// a storage error if the edge cannot be created.
    pub fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
        self.ensure_exists(from)?;
        self.ensure_exists(to)?;
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
        let decision = decision.trim();
        if decision.is_empty() {
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
