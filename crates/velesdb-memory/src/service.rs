//! The memory service: five operations over the in-core Agent Memory SDK.

use std::collections::{HashMap, HashSet};
#[cfg(feature = "persistence")]
use std::path::Path;

use serde_json::{Map, Value};

/// Structured metadata attached to a memory (the `ColumnStore` facet): exact-match
/// fields like `project`, `author`, `type`, `status`, `date`. `content` and
/// `_veles_expires_at` are reserved keys.
pub type Metadata = Map<String, Value>;

use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::extract::Extractor;
use crate::id;
use crate::model::{ColumnFilter, Explanation, Link, MemoryNode, Recollection};
#[cfg(feature = "persistence")]
use crate::storage::NativeStore;
use crate::storage::{is_reserved_key, strip_reserved_keys, MemoryStore};

/// [`MemoryService::recall_fused`] and its helpers — split out to keep this
/// file under the crate's 500-NLOC-per-file budget, same pattern as
/// `velesdb-core`'s `database/*.rs` split. A child module of `service`, so it
/// shares full access to `MemoryService`'s private fields and methods.
#[path = "fused_recall.rs"]
mod fused_recall;

/// [`MemoryService::feedback`] and the recall re-ranking it drives (RL Memory).
/// A child module of `service`, like [`fused_recall`], so it uses
/// `MemoryService`'s private `store` directly. Gated on `persistence`: it
/// builds on `velesdb-core`'s agent SDK (`ReinforcementStrategy`), itself
/// behind that feature, and a durable learned confidence is meaningless on the
/// in-memory (WASM) backend.
#[cfg(feature = "persistence")]
#[path = "reinforce.rs"]
mod reinforce;

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
/// Edge label a hub uses to point back at a fact it tags (the hub → fact
/// direction). [`fused_recall`] reads this to recognise which edges in a
/// `why()` walk crossed a hub, so it can weight the reached fact by that
/// hub's specificity instead of a flat constant.
const MENTIONS_RELATION: &str = "mentions";

/// Local-first agent memory backed by a single `VelesDB` instance.
///
/// Generic over the [`Embedder`] so production can use an on-device model while
/// tests use a deterministic, network-free one, and over the [`MemoryStore`]
/// backend `S` so the same orchestration runs over the native, file-backed
/// engine (the default — nothing changes for existing callers) or any other
/// backend that implements the trait (e.g. an in-memory one for WASM).
///
/// Two definitions, `persistence`-gated: the default type parameter itself
/// references [`NativeStore`], which doesn't exist as a type at all without
/// the feature, so a `persistence`-free build (e.g. `velesdb-wasm`) drops the
/// default and every caller names its own [`MemoryStore`] backend explicitly.
#[cfg(feature = "persistence")]
pub struct MemoryService<E: Embedder, S: MemoryStore = NativeStore> {
    store: S,
    embedder: E,
}
#[cfg(not(feature = "persistence"))]
pub struct MemoryService<E: Embedder, S: MemoryStore> {
    store: S,
    embedder: E,
}

#[cfg(feature = "persistence")]
impl<E: Embedder> MemoryService<E, NativeStore> {
    /// Open (or create) a native, file-backed memory store at `path`, using
    /// `embedder` for text vectorization. The store never leaves this directory.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the store cannot be opened or the agent
    /// memory cannot be initialized for the embedder's dimension.
    pub fn open<P: AsRef<Path>>(path: P, embedder: E) -> Result<Self, MemoryError> {
        let store = NativeStore::open(path, embedder.dimension())?;
        Ok(Self { store, embedder })
    }
}

impl<E: Embedder, S: MemoryStore> MemoryService<E, S> {
    /// Build a service directly over a `store` backend, bypassing
    /// [`Self::open`]'s filesystem-specific setup — the constructor a
    /// non-native backend (e.g. `velesdb-wasm`'s in-memory store) uses.
    pub fn with_store(store: S, embedder: E) -> Self {
        Self { store, embedder }
    }

    /// Remember a `fact`, optionally tagging it with structured `metadata`
    /// (`ColumnStore` facet) and linking it to existing memories (graph facet).
    /// Returns the stable id of the fact (idempotent on identical content).
    ///
    /// Every link is validated — target existence AND relation label —
    /// *before* the fact is stored, so bad link input never leaves the fact
    /// half-written. If an edge write itself fails afterwards (e.g. a target
    /// expiring concurrently), a freshly-created fact is rolled back; a
    /// re-remembered fact keeps its updated payload (re-remembering updates
    /// metadata by design, and deleting it would destroy prior state).
    /// Concurrent `remember`s of identical content are last-writer-wins,
    /// not transactional.
    ///
    /// # Errors
    /// Returns [`MemoryError::EmptyFact`] for empty/whitespace facts,
    /// [`MemoryError::ReservedKey`] if `metadata` names a reserved key
    /// (`content` or any `_veles_`-prefixed system key),
    /// [`MemoryError::UnknownMemory`] if a link points at a missing memory,
    /// [`MemoryError::InvalidRelation`] for a bad relation label,
    /// [`MemoryError::RollbackFailed`] if an edge write failed and the
    /// compensating delete also failed (the fact remains stored),
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
        // EVERY link property — relation label and target existence — is
        // validated before any write, so all deterministic link failures
        // happen while nothing has been stored or overwritten yet.
        for link in links {
            validate_relation(&link.relation)?;
        }
        self.ensure_link_targets_exist(links)?;
        let fact_id = id::stable_id(fact);
        let embedding = self.embedder.embed(fact)?;
        let existed_before = !links.is_empty() && self.store.get(fact_id)?.is_some();
        self.store_fact(
            fact_id,
            fact,
            &embedding,
            metadata,
            positive_ttl(ttl_seconds),
        )?;
        // Links are fully pre-validated above, so an edge write can only
        // fail here on a race (e.g. a target's TTL lapsing since the
        // pre-check). Roll a FRESH fact back (delete cascades any edges
        // already created); a fact that existed before this call is kept —
        // deleting it would destroy prior state, and its updated payload
        // stands per re-remember's update semantics. The existence probe
        // and the delete are not one atomic unit: a concurrent remember of
        // identical content between them is last-writer-wins (documented
        // on [`Self::remember`]).
        if let Err(e) = self.relate_links(fact_id, links) {
            if !existed_before {
                if let Err(rollback) = self.store.delete(fact_id) {
                    return Err(MemoryError::RollbackFailed {
                        cause: Box::new(e),
                        rollback: Box::new(rollback),
                    });
                }
            }
            return Err(e);
        }
        Ok(fact_id)
    }

    /// Create each outgoing link from `fact_id`.
    ///
    /// Precondition: every label was already validated by
    /// [`Self::remember_with_ttl`]'s pre-write pass (its only caller) —
    /// no re-check here, so the validation rule lives in exactly one
    /// place on this path.
    fn relate_links(&self, fact_id: u64, links: &[Link]) -> Result<(), MemoryError> {
        for link in links {
            self.store.relate(fact_id, link.target, &link.relation)?;
        }
        Ok(())
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
        self.add_edge(entity_id, fact_id, MENTIONS_RELATION, edges)?;
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
        for edge in self.store.relations(node)? {
            edges.insert((node, edge.to));
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
    /// it; goes straight to [`Self::store_fact`] to bypass the caller-facing
    /// reserved-key rejection in [`Self::remember`].
    fn remember_hub(&self, key: &str) -> Result<u64, MemoryError> {
        let id = id::stable_id(&format!("{HUB_ID_SALT}{key}"));
        let content = format!("Entity: {key}");
        let embedding = self.embedder.embed(&content)?;
        let mut meta = Map::new();
        meta.insert(HUB_FIELD.to_string(), Value::Bool(true));
        // Topic hubs are graph anchors — they never expire.
        self.store_fact(id, &content, &embedding, Some(&meta), None)?;
        Ok(id)
    }

    /// Fail with [`MemoryError::UnknownMemory`] unless memory `id` exists.
    fn ensure_exists(&self, id: u64) -> Result<(), MemoryError> {
        if self.store.get(id)?.is_none() {
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
    fn store_fact(
        &self,
        id: u64,
        fact: &str,
        embedding: &[f32],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        match (metadata, ttl_seconds) {
            (Some(meta), Some(ttl)) => {
                // store_with_ttl writes the fact + the durable expiry; update_metadata
                // then merges the metadata while preserving `_veles_expires_at`.
                self.store.store_with_ttl(id, fact, embedding, ttl)?;
                self.store.update_metadata(id, meta)?;
            }
            (Some(meta), None) => self.store.store_with_metadata(id, fact, embedding, meta)?,
            (None, Some(ttl)) => self.store.store_with_ttl(id, fact, embedding, ttl)?,
            (None, None) => self.store.store(id, fact, embedding)?,
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
    /// Each hit carries its caller metadata (`Recollection::metadata`, `None`
    /// when the fact carries none) — store a date field (e.g. `occurred_at`)
    /// and it round-trips here, so a caller can sort the result into a
    /// chronological, date-stamped context without `recall_where`'s explicit
    /// filters. One extra, single batched lookup covers every returned hit.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the semantic query or the metadata lookup fails.
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
        let ids: Vec<u64> = hits.iter().map(|(id, _, _)| *id).collect();
        // One raw batched payload lookup (reserved keys included), reused for
        // BOTH the RL re-rank and the caller-facing metadata below — a single
        // round trip, not one per concern.
        let payloads = self.store.get_metadata_batch(&ids)?;
        // RL Memory: re-order the recalled set by learned confidence. Facts
        // that never received `feedback` keep their similarity order exactly.
        #[cfg(feature = "persistence")]
        let (hits, payloads) = Self::rl_rerank(hits, payloads);
        Ok(hits
            .into_iter()
            .zip(payloads)
            .map(|((id, score, content), payload)| Recollection {
                id,
                score,
                content,
                metadata: strip_reserved_keys(payload),
            })
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
            // An include filter already excludes hubs: a hub's payload
            // carries only reserved keys (`content`, `_veles_hub`), and
            // reserved keys are rejected from caller filters, so a non-empty
            // filter can never match a hub. An EMPTY-but-present filter (`Some({})`, the
            // natural `{}` idiom at the JS boundary) matches every payload —
            // hubs included — so it must take the hub-excluding path below,
            // exactly like an absent filter (same `Some({})` ≡ `None`
            // convention as `recall_fused`'s graph-side `matches_filter`).
            Some(meta) if !meta.is_empty() => self.store.query_filtered(embedding, k, meta, 0),
            // Unfiltered recall must still drop entity hubs explicitly, or a hub
            // like `Entity: rust` would rank for the topic and evict a real fact.
            _ => self
                .store
                .query_excluding(embedding, k, &hub_exclude_filter()),
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
        // No column predicates = a plain recall: route through [`Self::recall`]
        // so entity hubs stay excluded — `query_columnar` with an empty filter
        // set is a bare vector search that would rank internal `Entity:` hub
        // scaffolding as results (same `[]` ≡ unfiltered convention as
        // `search`'s empty-map handling).
        if filters.is_empty() {
            return self.recall(query, k, None);
        }
        let embedding = self.embedder.embed(query)?;
        self.store.query_columnar(&embedding, k, filters)
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
        self.store.relate(from, to, relation)
    }

    /// Forget (delete) the memory with `fact_id`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the deletion fails.
    pub fn forget(&self, fact_id: u64) -> Result<(), MemoryError> {
        self.store.delete(fact_id)
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
        for edge in self.store.relations(node_id)? {
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

/// The metadata filter that excludes entity hubs from unfiltered recall and
/// `why` seeds — the negative counterpart [`MemoryService::search`] applies so
/// internal `_veles_hub` scaffolding never surfaces as a result.
fn hub_exclude_filter() -> Metadata {
    let mut exclude = Map::new();
    exclude.insert(HUB_FIELD.to_string(), Value::Bool(true));
    exclude
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
    if label.chars().any(|c| c.is_ascii_control()) {
        return Err(MemoryError::InvalidRelation(
            "relation label must not contain ASCII control characters".to_owned(),
        ));
    }
    Ok(())
}
