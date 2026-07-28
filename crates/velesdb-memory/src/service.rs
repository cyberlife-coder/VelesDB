//! The memory service: five operations over the in-core Agent Memory SDK.

use std::collections::{HashMap, HashSet};
#[cfg(feature = "persistence")]
use std::path::Path;

use serde_json::{Map, Value};

/// Structured metadata attached to a memory (the `ColumnStore` facet): exact-match
/// fields like `project`, `author`, `type`, `status`, `date`. `content` and
/// `_veles_expires_at` are reserved keys. [`crate::storage::AUTO_DATE_FIELD`]
/// (`_veles_date`) is auto-populated by [`MemoryService::remember_with_ttl`]
/// with today's date unless already present — see that method's docs.
pub type Metadata = Map<String, Value>;

use crate::clock;
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::extract::{ExtractedAttribute, ExtractedRelation, Extractor};
use crate::id;
use crate::model::{
    ColumnFilter, EntityProfile, EntityRelation, Explanation, Link, MemoryNode, Recollection,
    UnrelateOutcome,
};
#[cfg(feature = "persistence")]
use crate::storage::NativeStore;
use crate::storage::{is_reserved_key, strip_reserved_keys, MemoryStore, AUTO_DATE_FIELD};

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

/// The context compiler's memory bridge (`compile_context`,
/// `retrieve_context_source`, `context_savings`, working contexts). A child
/// module of `service`, like [`fused_recall`], so it reuses the private
/// `store_fact`/`HUB_FIELD` system-fact machinery — compiler system facts
/// (sources, events, working contexts) are hub-marked so they never surface
/// in normal recall.
#[cfg(feature = "context")]
#[path = "context/memory_bridge.rs"]
mod memory_bridge;

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
    autograph: Option<crate::extract::DynExtractor>,
}
#[cfg(not(feature = "persistence"))]
pub struct MemoryService<E: Embedder, S: MemoryStore> {
    store: S,
    embedder: E,
    autograph: Option<crate::extract::DynExtractor>,
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
        Ok(Self {
            store,
            embedder,
            autograph: None,
        })
    }
}

impl<E: Embedder, S: MemoryStore> MemoryService<E, S> {
    /// Build a service directly over a `store` backend, bypassing
    /// [`Self::open`]'s filesystem-specific setup — the constructor a
    /// non-native backend (e.g. `velesdb-wasm`'s in-memory store) uses.
    pub fn with_store(store: S, embedder: E) -> Self {
        Self {
            store,
            embedder,
            autograph: None,
        }
    }

    /// Turn on **autograph**: every [`Self::remember`] additionally reads the
    /// stored fact for entities, entity→entity edges and entity attributes,
    /// and wires them — so the knowledge graph builds itself from ordinary
    /// `remember` calls, with no separate [`Self::remember_extracted`].
    ///
    /// Opt-in, and off unless this is called. It costs one generation per
    /// `remember`, which is a real latency and availability change: a memory
    /// write that silently depends on a local model being up is not a default
    /// anyone should inherit.
    ///
    /// The caller's fact is stored **verbatim and first**. Autograph only
    /// *adds* structure around it; it never rewrites or replaces what the
    /// caller asked to remember.
    #[must_use]
    pub fn with_autograph(mut self, extractor: crate::extract::DynExtractor) -> Self {
        self.autograph = Some(extractor);
        self
    }

    /// Remember a `fact`, optionally tagging it with structured `metadata`
    /// (`ColumnStore` facet) and linking it to existing memories (graph facet).
    /// Returns the stable id of the fact (idempotent on identical content).
    ///
    /// The stored metadata is auto-stamped with today's date under
    /// [`crate::storage::AUTO_DATE_FIELD`] unless `metadata` already carries
    /// that key — see [`Self::remember_with_ttl`] (this method's only caller)
    /// for the full contract.
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
    /// [`MemoryError::FactTooLarge`] if the fact exceeds
    /// [`crate::limits::MAX_EMBEDDABLE_TEXT_BYTES`],
    /// [`MemoryError::SelfRelation`] if a link points the fact at itself,
    /// [`MemoryError::ReservedKey`] if `metadata` names a reserved key
    /// (`content` or any `_veles_`-prefixed system key, [`crate::storage::AUTO_DATE_FIELD`]
    /// excepted),
    /// [`MemoryError::MetadataTooLarge`] if `metadata` exceeds
    /// [`crate::limits::MAX_METADATA_BYTES`],
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
    /// expired facts stop being recalled. `None` stores the fact permanently,
    /// exactly like [`Self::remember`]; an explicit `Some(0)` is **refused**
    /// ([`MemoryError::ZeroTtl`]) rather than silently normalised to
    /// "permanent", which is the opposite of what a caller writing `0` means.
    /// Metadata and a TTL combine: the metadata is written and the expiry
    /// preserved.
    ///
    /// The stored metadata is **auto-stamped with today's date** under
    /// [`crate::storage::AUTO_DATE_FIELD`] (`_veles_date`, a `YYYYMMDD`
    /// integer read from the system clock at write time — see
    /// [`crate::clock::today_ymd`]) whenever `metadata` doesn't already carry
    /// that key; an explicit value in `metadata` (e.g. to date a fact
    /// retroactively) is never overwritten. No clock is available on
    /// `wasm32-unknown-unknown`, so that target stamps nothing and `metadata`
    /// passes through unchanged. This is the ONE place in the crate that
    /// reads wall-clock time on the write path — the context compiler
    /// (`compile_context` and friends) stays clock-free and deterministic,
    /// unaffected by this stamp (it never re-derives a date from `now()`,
    /// only ever reads whatever a fact already carries).
    ///
    /// Because [`Self::remember_extracted`] stores each extracted fact via
    /// [`Self::remember`] (which delegates here), it gets the same auto-stamp
    /// for free — entity hubs it also creates go through [`Self::store_fact`]
    /// directly and are never stamped, since they are internal graph
    /// scaffolding, not caller facts.
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
        self.remember_inner(fact, links, metadata, ttl_seconds, true)
    }

    /// The shared write path. `run_autograph` is false for the one caller that
    /// has ALREADY extracted the passage — [`Self::remember_extracted`] — so a
    /// service with autograph on does not run a second generation per stored
    /// fact, re-deriving what it just computed.
    fn remember_inner(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
        run_autograph: bool,
    ) -> Result<u64, MemoryError> {
        let fact = fact.trim();
        self.validate_write(fact, links, metadata, ttl_seconds)?;
        let fact_id = id::stable_id(fact);
        reject_self_links(fact_id, links)?;
        let existed_before = !links.is_empty() && self.store.get(fact_id)?.is_some();
        self.write_fact(fact_id, fact, metadata, ttl_seconds)?;
        self.link_or_rollback(fact_id, links, existed_before)?;
        self.autograph_if(run_autograph, fact_id, fact);
        Ok(fact_id)
    }

    /// Every deterministic rejection, before anything is written: a blank or
    /// over-long fact, an explicit zero TTL, reserved or oversized metadata,
    /// and each link's label and target. Run as one pass so a bad input never
    /// leaves a half-written fact behind.
    fn validate_write(
        &self,
        fact: &str,
        links: &[Link],
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        validate_fact(fact)?;
        reject_zero_ttl(ttl_seconds)?;
        reject_reserved_keys(metadata)?;
        reject_oversized_metadata(metadata)?;
        self.validate_links(links)
    }

    /// Embed the fact and persist it with its date-stamped metadata and TTL.
    fn write_fact(
        &self,
        fact_id: u64,
        fact: &str,
        metadata: Option<&Metadata>,
        ttl_seconds: Option<u64>,
    ) -> Result<(), MemoryError> {
        let embedding = self.embedder.embed(fact)?;
        let stamped = stamp_with_today(metadata);
        // `ttl_seconds` is already known positive-or-absent: `validate_write`
        // refuses an explicit `Some(0)` before any of this runs.
        self.store_fact(fact_id, fact, &embedding, stamped.as_ref(), ttl_seconds)
    }

    /// Validate EVERY link property — relation label and target existence —
    /// before any write, so all deterministic link failures happen while
    /// nothing has been stored or overwritten yet.
    fn validate_links(&self, links: &[Link]) -> Result<(), MemoryError> {
        for link in links {
            validate_relation(&link.relation)?;
        }
        self.ensure_link_targets_exist(links)
    }

    /// Write the edges, undoing a freshly-created fact if one of them fails.
    ///
    /// Links are fully pre-validated by [`Self::validate_links`], so an edge
    /// write can only fail here on a race (e.g. a target's TTL lapsing since
    /// the pre-check). Roll a FRESH fact back (delete cascades any edges
    /// already created); a fact that existed before the call is kept —
    /// deleting it would destroy prior state, and its updated payload stands
    /// per re-remember's update semantics. The existence probe and the delete
    /// are not one atomic unit: a concurrent remember of identical content
    /// between them is last-writer-wins (documented on [`Self::remember`]).
    fn link_or_rollback(
        &self,
        fact_id: u64,
        links: &[Link],
        existed_before: bool,
    ) -> Result<(), MemoryError> {
        let Err(cause) = self.relate_links(fact_id, links) else {
            return Ok(());
        };
        if existed_before {
            return Err(cause);
        }
        match self.store.delete(fact_id) {
            Ok(()) => Err(cause),
            Err(rollback) => Err(MemoryError::RollbackFailed {
                cause: Box::new(cause),
                rollback: Box::new(rollback),
            }),
        }
    }

    /// Run [`Self::autograph`] only when this write path asked for it — the
    /// branch lives here rather than in the write path itself.
    fn autograph_if(&self, run: bool, fact_id: u64, fact: &str) {
        if run {
            self.autograph(fact_id, fact);
        }
    }

    /// Autograph one just-stored fact: read the entities, entity→entity edges
    /// and attributes it states, and wire them around it.
    ///
    /// **Deliberately infallible.** The caller's fact is already durably
    /// stored by the time this runs, and the caller asked to remember a fact —
    /// not to run a model. Propagating an extraction failure would turn a
    /// successful write into a reported error, and an agent that sees
    /// `remember` fail will sensibly retry it, re-running the generation and
    /// failing again. So a model that is down, slow, or talking nonsense costs
    /// the *graph enrichment* and nothing else: the memory is kept, the id is
    /// returned, and the next `remember` tries again.
    ///
    /// The trade-off is that a persistently broken extractor degrades silently
    /// to plain `remember`. That is the right way round — losing structure is
    /// recoverable by re-remembering, losing the fact is not.
    fn autograph(&self, fact_id: u64, fact: &str) {
        let Some(extractor) = self.autograph.as_ref() else {
            return;
        };
        let Ok(mut extraction) = extractor.extract_graph(fact) else {
            return;
        };
        crate::extract::orient_possessive_kinship(fact, &mut extraction.relations);
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64)> = HashSet::new();
        let mut seeded: HashSet<u64> = HashSet::new();
        // The caller's fact is the node the topics attach to — the extracted
        // facts are NOT stored as separate memories here, which is what
        // separates autograph from `remember_extracted`: one `remember` call
        // must still produce exactly one caller-visible memory.
        for extracted in &extraction.facts {
            let _ = self.wire_entities(
                fact_id,
                &extracted.entities,
                &mut entity_ids,
                &mut edges,
                &mut seeded,
            );
        }
        let _ = self.wire_relations(
            &extraction.relations,
            &mut entity_ids,
            &mut edges,
            &mut seeded,
        );
        let _ = self.wire_attributes(&extraction.attributes, &mut entity_ids);
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
    /// if `metadata` names a reserved key, [`MemoryError::MetadataTooLarge`] if
    /// `metadata` exceeds [`crate::limits::MAX_METADATA_BYTES`], or a storage
    /// error if persistence fails.
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
        let mut extraction = extractor.extract_graph(text)?;
        crate::extract::orient_possessive_kinship(text, &mut extraction.relations);
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64)> = HashSet::new();
        let mut seeded: HashSet<u64> = HashSet::new();
        let fact_ids = self.store_extracted_facts(
            &extraction.facts,
            metadata,
            &mut entity_ids,
            &mut edges,
            &mut seeded,
        )?;
        self.wire_relations(
            &extraction.relations,
            &mut entity_ids,
            &mut edges,
            &mut seeded,
        )?;
        self.wire_attributes(&extraction.attributes, &mut entity_ids)?;
        Ok(fact_ids)
    }

    /// Look up everything known about a named entity: the attributes merged
    /// onto its hub, and the typed edges leaving it.
    ///
    /// This is the *read* side of the auto-built graph, and it exists because
    /// entity hubs are deliberately invisible to [`Self::recall`] and
    /// [`Self::recall_where`] — a hub ranking for its own topic would evict a
    /// real fact from the caller's results. Without this accessor an attribute
    /// merged onto a hub would be stored correctly and yet be unreachable
    /// through every public read path: the worst kind of feature, one that
    /// looks done and silently returns nothing.
    ///
    /// `name` is canonicalized exactly like an extracted entity (trimmed,
    /// lowercased), so the caller may pass `"Theo Durand"` and reach the node
    /// built from `"theo durand"`. Returns `None` when no hub exists for the
    /// name — nothing has ever mentioned that entity.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the store lookup fails.
    pub fn entity_profile(&self, name: &str) -> Result<Option<EntityProfile>, MemoryError> {
        let key = canonical_entity_name(name);
        if key.is_empty() {
            return Ok(None);
        }
        let id = id::stable_id(&format!("{HUB_ID_SALT}{key}"));
        if self.store.get(id)?.is_none() {
            return Ok(None);
        }
        // Reserved system keys (the hub flag itself) are scaffolding, not
        // attributes the caller ever wrote — strip them exactly as every other
        // caller-facing read path does.
        Ok(Some(EntityProfile {
            id,
            name: key,
            attributes: strip_reserved_keys(self.store.get_metadata(id)?).unwrap_or_default(),
            relations: self.outgoing_entity_relations(id)?,
        }))
    }

    /// The typed edges leaving `id`, resolved to their target's content.
    ///
    /// `mentions` edges are dropped: they point at the facts that tagged this
    /// entity, which is the bipartite scaffolding, not a statement *about* it.
    fn outgoing_entity_relations(&self, id: u64) -> Result<Vec<EntityRelation>, MemoryError> {
        let mut relations = Vec::new();
        for edge in self.store.relations(id)? {
            if edge.relation == MENTIONS_RELATION {
                continue;
            }
            let target = self.store.get(edge.to)?.map(|(content, _)| content);
            relations.push(EntityRelation {
                predicate: edge.relation,
                target_id: edge.to,
                target: target.unwrap_or_default(),
            });
        }
        Ok(relations)
    }

    /// Wire each extracted `subject -[predicate]-> object` triple as a typed
    /// edge between the two entity hubs.
    ///
    /// This is the step that turns the bipartite fact↔topic graph into a real
    /// knowledge graph. The hubs are resolved through [`Self::entity_hub`], so
    /// an endpoint naming an entity some earlier passage already introduced
    /// reuses that entity's existing node rather than forking a parallel one —
    /// hub ids are content-addressed, so this holds across calls and sessions.
    ///
    /// Only the stated direction is written. Inferring the converse
    /// (`father of` ⇒ `child of`) would mean inventing a label the passage
    /// never used, and an inverted vocabulary nobody can predict is worse than
    /// an absent edge: `why()` walks outgoing edges, so a wrong direction
    /// silently misroutes every later traversal.
    ///
    /// A malformed triple is skipped, not fatal — one unusable predicate must
    /// not cost the caller the facts stored alongside it.
    fn wire_relations(
        &self,
        relations: &[ExtractedRelation],
        entity_ids: &mut HashMap<String, u64>,
        edges: &mut HashSet<(u64, u64)>,
        seeded: &mut HashSet<u64>,
    ) -> Result<(), MemoryError> {
        for relation in relations {
            if validate_relation(&relation.predicate).is_err() {
                continue;
            }
            let subject_id = self.entity_hub(&relation.subject, entity_ids)?;
            let object_id = self.entity_hub(&relation.object, entity_ids)?;
            if subject_id == object_id {
                continue;
            }
            self.seed_existing_edges(subject_id, edges, seeded)?;
            self.add_edge(subject_id, object_id, &relation.predicate, edges)?;
        }
        Ok(())
    }

    /// Merge each extracted attribute into its entity hub's `ColumnStore`
    /// metadata, so `recall_where` can filter on it (`age >= 15`).
    ///
    /// The write goes through `update_metadata`, which **merges** rather than
    /// replaces. That is the whole point: learning "Theo has a sister" after
    /// "Theo is 15" must not erase the age. Re-storing the hub payload wholesale
    /// would silently drop every attribute learned in an earlier session.
    ///
    /// Values keep the JSON type the extractor produced. `recall_where`
    /// compares type-strictly with no coercion, so an age stored as `"15"`
    /// would never match a numeric filter — no error, just a permanent silent
    /// miss.
    ///
    /// Reserved keys are skipped: a model emitting `content` or a `_veles_`
    /// key must never be able to overwrite the hub's own content or its
    /// system flags.
    fn wire_attributes(
        &self,
        attributes: &[ExtractedAttribute],
        entity_ids: &mut HashMap<String, u64>,
    ) -> Result<(), MemoryError> {
        let mut per_entity: HashMap<String, Metadata> = HashMap::new();
        for attribute in attributes {
            if is_reserved_key(&attribute.key) {
                continue;
            }
            per_entity
                .entry(attribute.entity.clone())
                .or_default()
                .insert(attribute.key.clone(), attribute.value.clone());
        }
        for (entity, meta) in per_entity {
            if meta.is_empty() {
                continue;
            }
            reject_oversized_metadata(Some(&meta))?;
            let hub_id = self.entity_hub(&entity, entity_ids)?;
            self.store.update_metadata(hub_id, &meta)?;
        }
        Ok(())
    }

    /// Store each extracted fact and wire it to its topics, returning their ids.
    ///
    /// Goes through the no-autograph path: the passage was ALREADY extracted by
    /// the caller, so re-running a generation per stored fact would re-derive
    /// what was just computed.
    fn store_extracted_facts(
        &self,
        facts: &[crate::extract::ExtractedFact],
        metadata: Option<&Metadata>,
        entity_ids: &mut HashMap<String, u64>,
        edges: &mut HashSet<(u64, u64)>,
        seeded: &mut HashSet<u64>,
    ) -> Result<Vec<u64>, MemoryError> {
        let mut fact_ids = Vec::with_capacity(facts.len());
        for fact in facts {
            let content = fact.text.trim();
            if content.is_empty() {
                continue;
            }
            let fact_id = self.remember_inner(content, &[], metadata, None, false)?;
            fact_ids.push(fact_id);
            self.wire_entities(fact_id, &fact.entities, entity_ids, edges, seeded)?;
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
        // An existing hub is left exactly as it is. Re-storing it would rewrite
        // the payload to the bare hub marker and destroy every attribute merged
        // onto it by an earlier call — learning "Theo has a sister" would erase
        // "Theo is 15", because a later sentence re-resolves the same hub. The
        // content is a pure function of `key`, so there is nothing to refresh;
        // skipping also avoids re-embedding a hub on every single mention.
        if self.store.get(id)?.is_some() {
            return Ok(id);
        }
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
                // ONE write, not two. The previous `store_with_ttl` then
                // `update_metadata` pair left the fact live and expiring
                // between the calls: a short TTL could lapse in the gap and
                // the metadata write then failed with `NotFound(... is
                // expired ...)` — the caller got an error on a fact that was
                // valid when they asked for it. Observed with a 1 s TTL on a
                // loaded machine. Every TTL'd write takes this arm, since the
                // auto date stamp means `metadata` is always `Some`.
                self.store
                    .store_with_metadata_and_ttl(id, fact, embedding, meta, ttl)?;
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
    /// A self-loop (`from == to`) is refused: it states nothing, and `why`
    /// traverses it like any other edge, so it only adds noise to the
    /// evidence trail. The same rule covers [`Self::remember`]'s `links`.
    ///
    /// # Errors
    /// Returns [`MemoryError::InvalidRelation`] for a bad label,
    /// [`MemoryError::SelfRelation`] if both endpoints are the same memory,
    /// [`MemoryError::UnknownMemory`] if either endpoint is missing, or
    /// a storage error if the edge cannot be created.
    pub fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
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
    ) -> Result<UnrelateOutcome, MemoryError> {
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
    ) -> Result<usize, MemoryError> {
        let mut removed = 0usize;
        for edge in self.store.relations(from)? {
            if edge.to == to && edge.relation == relation && self.store.unrelate(edge.id)? {
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
    pub fn forget(&self, fact_id: u64) -> Result<bool, MemoryError> {
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
    fn hubs_linked_from(&self, fact_id: u64) -> Result<Vec<u64>, MemoryError> {
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
    fn collect_orphan_hubs(&self, hubs: &[u64]) -> Result<(), MemoryError> {
        for &hub in hubs {
            if !self.hub_still_mentioned(hub)? {
                self.store.delete(hub)?;
            }
        }
        Ok(())
    }

    /// Whether `hub` still points at a fact that exists.
    fn hub_still_mentioned(&self, hub: u64) -> Result<bool, MemoryError> {
        for edge in self.store.relations(hub)? {
            if edge.relation == MENTIONS_RELATION && self.store.get(edge.to)?.is_some() {
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

/// Reject caller-supplied metadata over [`crate::limits::MAX_METADATA_BYTES`]
/// — the `DoS` guard every `remember` path shares (see
/// [`MemoryError::MetadataTooLarge`]).
fn reject_oversized_metadata(metadata: Option<&Metadata>) -> Result<(), MemoryError> {
    let Some(meta) = metadata else {
        return Ok(());
    };
    let bytes = crate::limits::metadata_bytes(meta);
    if bytes > crate::limits::MAX_METADATA_BYTES {
        return Err(MemoryError::MetadataTooLarge {
            bytes,
            max: crate::limits::MAX_METADATA_BYTES,
        });
    }
    Ok(())
}

/// Normalise a TTL supplied as *configuration*: `Some(0)` (and `None`) mean
/// "no TTL policy" — the fact is stored permanently. Any positive value is
/// kept as-is.
///
/// Deliberately NOT applied to `remember`'s own `ttl_seconds` any more: an
/// explicit per-call `0` is an intent about one fact ("expire it"), and
/// silently turning that into "permanent" is the opposite (see
/// [`reject_zero_ttl`]). A compile policy's `source_ttl_seconds`, on the
/// other hand, is a knob about a whole server, where `0` reading as "no
/// policy" is the ordinary, unsurprising meaning.
///
/// Gated on `context`: since `remember` stopped calling it, the compile
/// policy in `context::memory_bridge` is its only caller, and a build
/// without that feature saw dead code — which `-D warnings` turns into a
/// failed build, not a warning.
#[cfg(feature = "context")]
pub(crate) fn positive_ttl(ttl_seconds: Option<u64>) -> Option<u64> {
    ttl_seconds.filter(|&seconds| seconds > 0)
}

/// The canonical form of an entity name: trimmed and lowercased, exactly as
/// an extracted entity is keyed.
///
/// Public because a lookup MISS has to echo it too: an adapter that answered
/// `name: ""` when nothing matched left a caller running several lookups
/// unable to pair a response with its question (issue #1654). Hit and miss go
/// through this one function, so the two can never drift.
#[must_use]
pub fn canonical_entity_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Refuse a fact that cannot be stored as written: blank, or past the size an
/// embedding model still accepts.
///
/// The size check runs BEFORE [`MemoryService::write_fact`] calls the
/// embedder, so an over-long fact is reported with its own size and the cap
/// instead of whatever the backend says — issue #1654 saw `ollama embeddings
/// call failed`, which names neither.
fn validate_fact(fact: &str) -> Result<(), MemoryError> {
    if fact.is_empty() {
        return Err(MemoryError::EmptyFact);
    }
    if fact.len() > crate::limits::MAX_EMBEDDABLE_TEXT_BYTES {
        return Err(MemoryError::FactTooLarge {
            bytes: fact.len(),
            max: crate::limits::MAX_EMBEDDABLE_TEXT_BYTES,
        });
    }
    Ok(())
}

/// Refuse an explicit per-call TTL of `0`.
///
/// `0` used to be normalised to "no expiry", so a caller who meant "expire
/// immediately" silently got a **permanent** fact — the opposite intent, with
/// no signal (issue #1654). A TTL supplied as *configuration*
/// (`McpServer::with_default_ttl`, a compile policy's `source_ttl_seconds`)
/// still reads `0` as "no TTL policy": that is a default about a whole
/// server, not an intent about one fact, and it is deliberately untouched.
fn reject_zero_ttl(ttl_seconds: Option<u64>) -> Result<(), MemoryError> {
    if ttl_seconds == Some(0) {
        return Err(MemoryError::ZeroTtl);
    }
    Ok(())
}

/// Refuse a `remember` link that points the fact at itself.
///
/// The same rule [`MemoryService::relate`] enforces, applied to the other way
/// a self-loop can be created: re-remembering existing content yields its
/// existing id, so a caller CAN name that id as a link target. Without this
/// the `relate` guard would only close half the door.
fn reject_self_links(fact_id: u64, links: &[Link]) -> Result<(), MemoryError> {
    if links.iter().any(|link| link.target == fact_id) {
        return Err(MemoryError::SelfRelation(fact_id));
    }
    Ok(())
}

/// [`MemoryService::remember_with_ttl`]'s auto-date stamp: `metadata` with
/// today's date added under [`AUTO_DATE_FIELD`], unless `metadata` already
/// names that key (an explicit, possibly retroactive, caller value is never
/// overwritten) or no clock is available ([`clock::today_ymd`] returns `None`
/// on `wasm32-unknown-unknown`). Returns an owned map either way, `None` only
/// when there is nothing to store at all (no caller metadata AND no clock).
fn stamp_with_today(metadata: Option<&Metadata>) -> Option<Metadata> {
    if metadata.is_some_and(|meta| meta.contains_key(AUTO_DATE_FIELD)) {
        return metadata.cloned();
    }
    let Some(today) = clock::today_ymd() else {
        return metadata.cloned();
    };
    let mut stamped = metadata.cloned().unwrap_or_default();
    stamped.insert(AUTO_DATE_FIELD.to_owned(), Value::from(today));
    Some(stamped)
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
