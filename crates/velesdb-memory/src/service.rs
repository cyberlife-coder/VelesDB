//! The memory service: five operations over the in-core Agent Memory SDK.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use velesdb_core::agent::AgentMemory;
use velesdb_core::{Database, SearchResult};

/// Structured metadata attached to a memory (the `ColumnStore` facet): exact-match
/// fields like `project`, `author`, `type`, `status`, `date`. `content` and
/// `_veles_expires_at` are reserved keys.
pub type Metadata = Map<String, Value>;

use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::extract::Extractor;
use crate::id;
use crate::model::{ColumnFilter, Explanation, Link, MemoryEdge, MemoryNode, Recollection};

/// Reserved metadata key marking an entity hub auto-created by
/// [`MemoryService::remember_extracted`] (value `true`). Namespaced under the
/// system `_veles_` prefix so it can never collide with a caller's own metadata,
/// and rejected from caller-supplied metadata/filters (see [`is_reserved_key`]).
/// Hubs are internal graph scaffolding — they connect facts that share a topic —
/// so they are excluded from unfiltered recall and from `why` seeds.
const HUB_FIELD: &str = "_veles_hub";
/// Salt mixed into a hub's stable id so the hub id space is disjoint from
/// natural fact ids: a caller fact whose text happens to equal a hub's display
/// content (`Entity: rust`) can never collide with, or overwrite, the hub.
const HUB_ID_SALT: &str = "\u{0}_veles_entity_hub\u{0}";

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
    /// [`MemoryError::ReservedKey`] if `metadata` names a reserved key
    /// (`content` or any `_veles_`-prefixed system key),
    /// [`MemoryError::UnknownMemory`] if a link points at a missing memory,
    /// or a storage error if persistence fails.
    pub fn remember(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
    ) -> Result<u64, MemoryError> {
        self.remember_with_ttl(fact, links, metadata, None)
    }

    /// Like [`Self::remember`], but the fact **expires after `ttl_seconds`**.
    ///
    /// The expiry is a durable TTL — persisted with the fact (reserved
    /// `_veles_expires_at` payload field), so it survives a process restart, and
    /// expired facts stop being recalled. `None` (or `Some(0)`) stores the fact
    /// permanently, exactly like [`Self::remember`]. Metadata and a TTL combine:
    /// the metadata is written and the expiry preserved.
    ///
    /// # Errors
    /// Same as [`Self::remember`].
    pub fn remember_with_ttl(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
    ) -> Result<u64, MemoryError> {
        let fact = fact.trim();
        if fact.is_empty() {
            return Err(MemoryError::EmptyFact);
        }
        reject_reserved_keys(metadata)?;
        self.ensure_link_targets_exist(links)?;
        let fact_id = id::stable_id(fact);
        let embedding = self.embedder.embed(fact)?;
        self.store(
            fact_id,
            fact,
            &embedding,
            metadata,
            positive_ttl(ttl_seconds),
        )?;
        for link in links {
            validate_relation(&link.relation)?;
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
    /// [`MemoryError::Extract`] if extraction fails, [`MemoryError::ReservedKey`]
    /// if `metadata` names a reserved key, or a storage error if persistence fails.
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
        let mut fact_ids = Vec::with_capacity(facts.len());
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64)> = HashSet::new();
        let mut seeded: HashSet<u64> = HashSet::new();
        for fact in &facts {
            let content = fact.text.trim();
            if content.is_empty() {
                continue;
            }
            let fact_id = self.remember(content, &[], metadata)?;
            fact_ids.push(fact_id);
            self.wire_entities(
                fact_id,
                &fact.entities,
                &mut entity_ids,
                &mut edges,
                &mut seeded,
            )?;
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
        seeded: &mut HashSet<u64>,
    ) -> Result<(), MemoryError> {
        for entity in entities {
            // Skip blank or punctuation-only topics: they would persist as junk
            // hubs (`Entity: -`) yet can never carry a meaningful multi-hop link.
            if entity.chars().any(char::is_alphanumeric) {
                self.wire_entity(fact_id, entity, entity_ids, edges, seeded)?;
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
        seeded: &mut HashSet<u64>,
    ) -> Result<(), MemoryError> {
        let entity_id = self.entity_hub(entity, entity_ids)?;
        if entity_id == fact_id {
            return Ok(());
        }
        // Fold already-persisted edges into the dedup set so re-ingesting the
        // same text never creates duplicate parallel edges (core `relate` does
        // not dedup by endpoint+label, only by edge id).
        self.seed_existing_edges(fact_id, edges, seeded)?;
        self.seed_existing_edges(entity_id, edges, seeded)?;
        self.add_edge(fact_id, entity_id, "about", edges)?;
        self.add_edge(entity_id, fact_id, "mentions", edges)?;
        Ok(())
    }

    /// Create the edge `from -> to` labelled `label`, unless `edges` already
    /// records that endpoint pair (in-call and persisted dedup).
    fn add_edge(
        &self,
        from: u64,
        to: u64,
        label: &str,
        edges: &mut HashSet<(u64, u64)>,
    ) -> Result<(), MemoryError> {
        if edges.insert((from, to)) {
            self.relate(from, to, label)?;
        }
        Ok(())
    }

    /// Load `node`'s already-persisted outgoing edges into `edges` once per call
    /// (tracked by `seeded`), so the dedup set reflects the stored graph and a
    /// repeated ingest is idempotent rather than edge-duplicating.
    fn seed_existing_edges(
        &self,
        node: u64,
        edges: &mut HashSet<(u64, u64)>,
        seeded: &mut HashSet<u64>,
    ) -> Result<(), MemoryError> {
        if !seeded.insert(node) {
            return Ok(());
        }
        for edge in self.memory.semantic().relations(node)? {
            edges.insert((node, edge.target()));
        }
        Ok(())
    }

    /// Get or create the hub memory for a topic, caching its id per call. The
    /// hub id is a deterministic function of the (normalized) topic, so the same
    /// topic resolves to the same hub across calls — never a duplicate.
    fn entity_hub(
        &self,
        entity: &str,
        entity_ids: &mut HashMap<String, u64>,
    ) -> Result<u64, MemoryError> {
        let key = entity.trim().to_lowercase();
        if let Some(&id) = entity_ids.get(&key) {
            return Ok(id);
        }
        let id = self.remember_hub(&key)?;
        entity_ids.insert(key, id);
        Ok(id)
    }

    /// Idempotently store the hub memory for topic `key`. The id is salted so the
    /// hub id space is disjoint from natural fact ids (no caller fact can collide
    /// with or overwrite a hub), while the stored content stays human-readable.
    /// Marked with the reserved [`HUB_FIELD`] so recall and `why` seeds exclude
    /// it; goes straight to [`Self::store`] to bypass the caller-facing reserved-
    /// key rejection in [`Self::remember`].
    fn remember_hub(&self, key: &str) -> Result<u64, MemoryError> {
        let id = id::stable_id(&format!("{HUB_ID_SALT}{key}"));
        let content = format!("Entity: {key}");
        let embedding = self.embedder.embed(&content)?;
        let mut meta = Map::new();
        meta.insert(HUB_FIELD.to_string(), Value::Bool(true));
        // Topic hubs are graph anchors — they never expire.
        self.store(id, &content, &embedding, Some(&meta), None)?;
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

    /// Store a fact with any combination of metadata and a durable TTL.
    fn store(
        &self,
        id: u64,
        fact: &str,
        embedding: &[f32],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let semantic = self.memory.semantic();
        match (metadata, ttl_seconds) {
            (Some(meta), Some(ttl)) => {
                // store_with_ttl writes the fact + the durable expiry; update_metadata
                // then merges the metadata while preserving `_veles_expires_at`.
                semantic.store_with_ttl(id, fact, embedding, ttl)?;
                semantic.update_metadata(id, meta)?;
            }
            (Some(meta), None) => semantic.store_with_metadata(id, fact, embedding, meta)?,
            (None, Some(ttl)) => semantic.store_with_ttl(id, fact, embedding, ttl)?,
            (None, None) => semantic.store(id, fact, embedding)?,
        }
        Ok(())
    }

    /// Recall up to `k` memories semantically similar to `query` (vector facet),
    /// optionally narrowed to an exact-match metadata `filter` (`ColumnStore`
    /// facet) — e.g. `{ "project": "veles", "status": "resolved" }`.
    ///
    /// A highly selective filter may return fewer than `k` hits even when more
    /// matches exist — raise `k` for fuller coverage with a narrow filter.
    ///
    /// Entity hubs created by [`Self::remember_extracted`] are never returned:
    /// they are internal graph scaffolding, not facts the caller stored.
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
        reject_reserved_keys(filter)?;
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
            // An include filter already excludes hubs: a hub only carries
            // `{kind: entity}`, so it can never match a user's metadata filter.
            Some(meta) => self
                .memory
                .semantic()
                .query_filtered(embedding, k, meta, 0)
                .map_err(MemoryError::from),
            // Unfiltered recall must still drop entity hubs explicitly, or a hub
            // like `Entity: rust` would rank for the topic and evict a real fact.
            None => self
                .memory
                .semantic()
                .query_excluding(embedding, k, &hub_exclude_filter())
                .map_err(MemoryError::from),
        }
    }

    /// Fused recall: semantic `NEAR` search combined with structured
    /// `ColumnStore` predicates over metadata columns — ranges and comparisons,
    /// not just the equality of [`Self::recall`]. One query spanning the vector
    /// and column facets (e.g. "most similar facts **with `timestamp` in this
    /// window**"), which a vector-only or equality-only recall cannot express.
    ///
    /// Filter *values* are bound as query parameters (never interpolated), so
    /// they cannot inject; filter *field names* are validated to be plain
    /// identifiers. Results come back in similarity order.
    ///
    /// # Errors
    /// Returns [`MemoryError::InvalidFilter`] if a filter field is not a plain
    /// identifier, [`MemoryError::Embed`] if the query cannot be embedded, or a
    /// storage error if the query fails. An empty query or `k == 0` yields `[]`.
    pub fn recall_where(
        &self,
        query: &str,
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError> {
        let query = query.trim();
        if query.is_empty() || k == 0 {
            return Ok(Vec::new());
        }
        let embedding = self.embedder.embed(query)?;
        let (sql, params) = self.build_fused_query(&embedding, k, filters)?;
        // `build_fused_query` has validated every field name; ensure each one is
        // indexed so the planner uses a bitmap prefilter instead of an O(n)
        // post-filter scan. Idempotent and incrementally maintained thereafter.
        for filter in filters {
            self.memory
                .semantic()
                .ensure_index(&filter.field)
                .map_err(MemoryError::from)?;
        }
        let results = self
            .memory
            .query_semantic(&sql, &params)
            .map_err(MemoryError::from)?;
        Ok(results.iter().map(to_recollection).collect())
    }

    /// Build the `VelesQL` for [`Self::recall_where`]: a `NEAR` predicate plus
    /// one bound parameter per filter, against the semantic collection.
    fn build_fused_query(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<(String, HashMap<String, Value>), MemoryError> {
        use std::fmt::Write as _;
        let mut params: HashMap<String, Value> = HashMap::new();
        params.insert("q".to_string(), json!(embedding));
        let mut predicate = String::from("vector NEAR $q");
        for (index, filter) in filters.iter().enumerate() {
            validate_field(&filter.field)?;
            validate_scalar(&filter.value)?;
            let key = format!("p{index}");
            // Field is a validated identifier; the value is bound, never inlined.
            let _ = write!(
                predicate,
                " AND {} {} ${key}",
                filter.field,
                filter.op.as_sql()
            );
            params.insert(key, filter.value.clone());
        }
        let sql = format!(
            "SELECT * FROM {} WHERE {predicate} LIMIT {k}",
            self.memory.semantic().collection_name()
        );
        Ok((sql, params))
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
        validate_relation(relation)?;
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
        let mut next: Vec<u64> = Vec::new();
        for hop in 1..=max_hops {
            next.clear();
            for node_id in frontier.drain(..) {
                self.expand(node_id, hop, &mut explanation, &mut visited, &mut next)?;
            }
            if next.is_empty() {
                break;
            }
            std::mem::swap(&mut frontier, &mut next);
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

/// The metadata filter that excludes entity hubs from unfiltered recall and
/// `why` seeds — the negative counterpart [`MemoryService::search`] applies so
/// internal `_veles_hub` scaffolding never surfaces as a result.
fn hub_exclude_filter() -> Metadata {
    let mut exclude = Map::new();
    exclude.insert(HUB_FIELD.to_string(), Value::Bool(true));
    exclude
}

/// True for metadata keys the memory layer reserves: the engine's `content`
/// payload, and any `_veles_`-namespaced system key (durable TTL, entity hubs).
/// Callers may neither set these in `remember` metadata nor filter on them, so
/// they can't overwrite a system field or collide with internal scaffolding.
fn is_reserved_key(key: &str) -> bool {
    key == "content" || key.starts_with("_veles_")
}

/// Reject caller-supplied metadata/filters that name a reserved key.
fn reject_reserved_keys(metadata: Option<&Metadata>) -> Result<(), MemoryError> {
    let Some(meta) = metadata else {
        return Ok(());
    };
    for key in meta.keys() {
        if is_reserved_key(key) {
            return Err(MemoryError::ReservedKey(key.clone()));
        }
    }
    Ok(())
}

/// Normalise a requested TTL: `Some(0)` (and `None`) mean "no expiry" — the fact
/// is stored permanently. Any positive value is kept as-is.
fn positive_ttl(ttl_seconds: Option<u64>) -> Option<u64> {
    ttl_seconds.filter(|&seconds| seconds > 0)
}

/// Map a core search result to a [`Recollection`], lifting the fact text out of
/// the reserved `content` payload key.
fn to_recollection(result: &SearchResult) -> Recollection {
    let content = result
        .point
        .payload
        .as_ref()
        .and_then(|payload| payload.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Recollection {
        id: result.point.id,
        score: result.score,
        content,
    }
}

/// Accept only plain, non-reserved identifier field names, so a filter field
/// can be safely placed into the query text (its value is always a bound
/// parameter). Rejects the reserved system columns the docs promise are off
/// limits: `content` (the fact payload) and any `_veles_`-prefixed engine key
/// (e.g. durable TTL).
fn validate_field(field: &str) -> Result<(), MemoryError> {
    let plain = !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let reserved = field == "content" || field.starts_with("_veles_");
    if plain && !reserved {
        Ok(())
    } else {
        Err(MemoryError::InvalidFilter(field.to_owned()))
    }
}

/// Reject non-scalar filter values. Only strings, numbers, and booleans can be
/// compared against a `ColumnStore` column; binding an array/object/null would
/// fail deep in the query engine and surface as an opaque internal error instead
/// of a clear client-input error.
fn validate_scalar(value: &Value) -> Result<(), MemoryError> {
    match value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
        _ => Err(MemoryError::InvalidFilter(format!(
            "value must be a string, number, or boolean, got {value}"
        ))),
    }
}

/// Maximum byte length for a relation label (prevents oversized graph edge labels
/// from reaching the storage layer).
const MAX_RELATION_BYTES: usize = 512;

/// Validate a caller-supplied relation label: non-empty, within the size cap, and
/// containing only printable, non-control ASCII characters (32–126) or non-ASCII
/// Unicode. This prevents null bytes and control characters from reaching the
/// storage layer while permitting natural-language labels like `"decided_in"` or
/// `"is a friend of"`.
fn validate_relation(label: &str) -> Result<(), MemoryError> {
    if label.is_empty() {
        return Err(MemoryError::InvalidRelation(
            "relation label must not be empty".to_owned(),
        ));
    }
    if label.len() > MAX_RELATION_BYTES {
        return Err(MemoryError::InvalidRelation(format!(
            "relation label exceeds maximum of {MAX_RELATION_BYTES} bytes ({} given)",
            label.len()
        )));
    }
    if label.chars().any(|c| c.is_ascii() && c.is_ascii_control()) {
        return Err(MemoryError::InvalidRelation(
            "relation label must not contain ASCII control characters".to_owned(),
        ));
    }
    Ok(())
}
