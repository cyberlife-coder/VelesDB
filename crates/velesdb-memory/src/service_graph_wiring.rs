//! The *construction* half of [`MemoryService`]'s graph surface — the
//! `remember*`/`autograph*` family and the `wire_*`/`store_extract*`
//! plumbing that turns facts and extractions into hub nodes and edges.
//! Split out of `service.rs` under the file-budget rule, completing the cut
//! `service_graph.rs` started (#2021): every `S: GraphStore`-bounded method
//! now lives in one of the two `service_graph*` modules — walks and
//! destruction there, wiring here (either alone would blow the same budget
//! the split serves). A `#[path]` child of `service`, sharing full access
//! to `MemoryService`'s private fields and methods.

use crate::id;

use super::{
    canonical_entity_name, reject_self_links, strip_reserved_keys, validate_relation, AutographJob,
    Embedder, EntityProfile, EntityRelation, ExtractedRelation, Extractor, FactStore, GraphStore,
    HashMap, HashSet, Link, MemoryError, MemoryService, Metadata, RememberedExtraction,
    ABOUT_RELATION, HUB_ID_SALT, MENTIONS_RELATION,
};

// Only the spawned-worker pair at the bottom touches this; that whole impl
// block is compiled out on wasm32, so the import is gated with it.
#[cfg(not(target_arch = "wasm32"))]
use super::AutographWorkerHandle;

/// The three wiring stages of one autograph enrichment, in the order they
/// run. Named for the failure log: which stage a partial enrichment stopped
/// at is the one fact a reader needs to judge what is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutographStage {
    /// Fact → topic hubs (`about`/`mentions` pairs).
    Entities,
    /// Hub → hub typed edges.
    Relations,
    /// Hub attribute writes.
    Attributes,
}

impl std::fmt::Display for AutographStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Entities => "entities",
            Self::Relations => "relations",
            Self::Attributes => "attributes",
        })
    }
}

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    // Autograph observability lives with autograph (this module), not in the
    // service assembler: `service.rs` is over the file budget and only ever
    // shrinks (scripts/check-file-budgets.py), so the counters' accessors
    // moved here beside the worker that feeds them.

    /// How many autograph enrichments a FULL queue refused since this
    /// service was built (#1846). The facts themselves were stored; only
    /// their graph wiring was skipped, and re-remembering a fact rebuilds it.
    #[must_use]
    pub fn autograph_dropped(&self) -> u64 {
        self.autograph_queue
            .dropped
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How many autograph enrichments RAN and failed part-way through
    /// wiring since this service was built. Distinct from
    /// [`Self::autograph_dropped`], which counts the ones a full queue never
    /// ran. A failure here leaves the fact stored and its graph structure
    /// PARTIAL — the entities wired before the failing write are in, the
    /// rest are not — and re-remembering the fact completes it. Until this
    /// counter existed such failures were discarded with `let _`: not
    /// counted, not logged, invisible to `memory_status`, while the doc
    /// promised "never silent" — a promise that covered only the queue.
    #[must_use]
    pub fn autograph_failed(&self) -> u64 {
        self.autograph_queue
            .failed
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether the background autograph queue is OPEN — a worker is spawned
    /// and `remember` enqueues instead of running the enrichment inline.
    /// Turns false the moment a worker handle's drop closes the queue.
    #[must_use]
    pub fn autograph_queue_open(&self) -> bool {
        self.autograph_queue.tx.lock().is_some()
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
    ) -> Result<u64, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        self.remember_inner(fact, links, metadata, None, true)
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
    /// `clock::today_ymd`) whenever `metadata` doesn't already carry
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
    /// for free — entity hubs it also creates go through `store_fact`
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
    ) -> Result<u64, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
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
    ) -> Result<u64, MemoryError>
    where
        S: GraphStore,
    {
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
    ) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
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
    ///
    /// With a worker spawned ([`Self::spawn_autograph_worker`]), the job is
    /// ENQUEUED and this returns immediately: the enrichment leaves the
    /// caller's response path (#1846). A FULL queue drops the job, counted
    /// in [`Self::autograph_dropped`] — losing structure is recoverable by
    /// re-remembering, stalling every write behind a slow model is not. A
    /// disconnected queue (worker gone) falls back inline, so the graph
    /// keeps building even if the worker died.
    fn autograph_if(&self, run: bool, fact_id: u64, fact: &str)
    where
        S: GraphStore,
    {
        if !run {
            return;
        }
        let guard = self.autograph_queue.tx.lock();
        if let Some(tx) = guard.as_ref() {
            use std::sync::mpsc::TrySendError;
            match tx.try_send(AutographJob {
                fact_id,
                fact: fact.to_owned(),
            }) {
                Ok(()) => return,
                Err(TrySendError::Full(_)) => {
                    self.autograph_queue
                        .dropped
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    #[cfg(feature = "mcp")]
                    tracing::warn!(
                        fact_id,
                        "autograph queue full: enrichment dropped — the fact is \
                         stored, its graph structure is not; re-remembering \
                         rebuilds it"
                    );
                    return;
                }
                Err(TrySendError::Disconnected(_)) => {
                    // fall through to the inline path below
                }
            }
        }
        drop(guard);
        self.autograph(fact_id, fact);
    }

    /// The total number of graph edges, when the backend can say —
    /// [`GraphStore::edge_count`], relayed for `memory_status`. `None`
    /// means "cannot say", never "zero": the two answers tell a caller
    /// different things about `why()`.
    #[must_use]
    pub fn edge_count(&self) -> Option<usize>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        self.store.edge_count()
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
    pub(super) fn autograph(&self, fact_id: u64, fact: &str)
    where
        S: GraphStore,
    {
        let Some(extractor) = self.autograph.as_ref() else {
            return;
        };
        let Ok(mut extraction) = extractor.extract_graph(fact) else {
            return;
        };
        // Privacy invariant: autograph derives structure FROM a stored fact, so
        // if the fact no longer exists, none of its derived structure may be
        // created. With a background worker a job can sit queued for
        // minutes-to-hours and its generation runs for tens of seconds, so a
        // `forget` issued in between must win — a permanent deletion cannot be
        // undone by a stale enrichment resurrecting the entity hubs. Re-check
        // once the generation has returned, before any wiring: this closes the
        // whole queue-plus-generation window, the common case, completely.
        if !self.fact_exists(fact_id) {
            return;
        }
        crate::extract::orient_kinship(fact, &mut extraction.relations);
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64, String)> = HashSet::new();
        // The caller's fact is the node the topics attach to — the extracted
        // facts are NOT stored as separate memories here, which is what
        // separates autograph from `remember_extracted`: one `remember` call
        // must still produce exactly one caller-visible memory.
        for extracted in &extraction.facts {
            if let Err(err) =
                self.wire_entities(fact_id, &extracted.entities, &mut entity_ids, &mut edges)
            {
                self.note_autograph_failure(fact_id, AutographStage::Entities, &err);
            }
        }
        // A concurrent `forget` can still race the wiring writes above between
        // the first check and here. Re-check once more before the hub↔hub and
        // attribute writes — neither of which references the source fact, so
        // `ensure_exists` cannot catch a retired fact for them — leaving a
        // residual window of a single already-committed generation, not the
        // whole job.
        if !self.fact_exists(fact_id) {
            return;
        }
        if let Err(err) = self.wire_relations(&extraction.relations, &mut entity_ids, &mut edges) {
            self.note_autograph_failure(fact_id, AutographStage::Relations, &err);
        }
        if let Err(err) = self.wire_attributes(&extraction.attributes, &mut entity_ids) {
            self.note_autograph_failure(fact_id, AutographStage::Attributes, &err);
        }
    }

    /// One enrichment ran and a wiring write failed part-way: count it and
    /// say so. The wiring helpers propagate with `?`, so the first failing
    /// write ends its stage; the fact is stored and its graph structure is
    /// partial until it is re-remembered. Discarding the error here — as
    /// `let _` did — left that invisible everywhere: no counter, no log, and
    /// a `memory_status` that reported a healthy worker. One line per failed
    /// stage, not per edge (#1834's rule): the concurrent-`forget` race the
    /// comments above describe is the common cause, and it fails the whole
    /// stage at once.
    fn note_autograph_failure(&self, fact_id: u64, stage: AutographStage, err: &MemoryError) {
        self.autograph_queue
            .failed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        #[cfg(feature = "mcp")]
        tracing::warn!(
            fact_id,
            stage = %stage,
            error = %err,
            "autograph enrichment failed part-way: the fact is stored, its graph \
             structure is partial; re-remembering it completes the wiring"
        );
        #[cfg(not(feature = "mcp"))]
        let _ = (fact_id, stage, err);
    }

    /// Create each outgoing link from `fact_id`.
    ///
    /// Precondition: every label was already validated by
    /// [`Self::remember_with_ttl`]'s pre-write pass (its only caller) —
    /// no re-check here, so the validation rule lives in exactly one
    /// place on this path.
    fn relate_links(&self, fact_id: u64, links: &[Link]) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
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
    /// hubs excluded), in extraction order, plus how many facts were skipped
    /// for exceeding the embeddable cap — one unusable fact must not cost the
    /// others, the policy every other stage of this pipeline already follows
    /// (a malformed triple is skipped, a blank entity is skipped).
    ///
    /// # Errors
    /// Returns [`MemoryError::EmptyFact`] for empty/whitespace `text`,
    /// [`MemoryError::Extract`] if extraction fails, [`MemoryError::ReservedKey`]
    /// if `metadata` names a reserved key, [`MemoryError::MetadataTooLarge`] if
    /// `metadata` exceeds [`crate::limits::MAX_METADATA_BYTES`], or a storage
    /// error if persistence fails. A fact past
    /// [`crate::limits::MAX_EMBEDDABLE_TEXT_BYTES`] is NOT an error: it is
    /// counted in [`RememberedExtraction::skipped_over_cap`] and the call
    /// carries on.
    pub fn remember_extracted<X: Extractor>(
        &self,
        text: &str,
        extractor: &X,
        metadata: Option<&Metadata>,
    ) -> Result<RememberedExtraction, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        let extraction = Self::extract_passage(text, extractor)?;
        self.store_extraction_inner(&extraction, metadata)
    }

    /// Store a previously generated extraction through the same idempotent
    /// fact, hub, edge, and attribute primitives as [`Self::remember_extracted`].
    #[cfg(feature = "mcp")]
    pub(crate) fn store_extraction(
        &self,
        extraction: &crate::extract::Extraction,
        metadata: Option<&Metadata>,
    ) -> Result<RememberedExtraction, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
        self.store_extraction_inner(extraction, metadata)
    }

    fn store_extraction_inner(
        &self,
        extraction: &crate::extract::Extraction,
        metadata: Option<&Metadata>,
    ) -> Result<RememberedExtraction, MemoryError>
    where
        S: GraphStore,
    {
        let mut entity_ids: HashMap<String, u64> = HashMap::new();
        let mut edges: HashSet<(u64, u64, String)> = HashSet::new();
        let outcome =
            self.store_extracted_facts(&extraction.facts, metadata, &mut entity_ids, &mut edges)?;
        self.wire_relations(&extraction.relations, &mut entity_ids, &mut edges)?;
        self.wire_attributes(&extraction.attributes, &mut entity_ids)?;
        Ok(outcome)
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
    pub fn entity_profile(&self, name: &str) -> Result<Option<EntityProfile>, MemoryError>
    where
        S: GraphStore,
    {
        let _generation = self.enter_generation();
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
        let (relations, relations_truncated) = self.outgoing_entity_relations(id)?;
        let (relations_in, relations_in_truncated) = self.incoming_entity_relations(id)?;
        Ok(Some(EntityProfile {
            id,
            name: key,
            attributes: strip_reserved_keys(self.store.get_metadata(id)?).unwrap_or_default(),
            relations,
            relations_in,
            relations_truncated,
            relations_in_truncated,
        }))
    }

    /// The typed edges leaving `id`, resolved to their target's content, and
    /// whether that list is a partial view.
    ///
    /// Scaffolding edges (`mentions`, and `about` for symmetry — a hub never
    /// has an outgoing `about`) are dropped: they point at the facts that
    /// tagged this entity, not at a statement *about* it.
    fn outgoing_entity_relations(&self, id: u64) -> Result<(Vec<EntityRelation>, bool), MemoryError>
    where
        S: GraphStore,
    {
        let scanned = self
            .store
            .relations_bounded(id, crate::limits::MAX_ENTITY_SCAN_EDGES)?;
        self.resolve_entity_relations(scanned, |edge| edge.to)
    }

    /// The typed edges pointing at `id`, resolved to their SOURCE's content —
    /// for an incoming edge the far end is where it comes *from* — and
    /// whether that list is a partial view.
    ///
    /// Without these, a question is only answerable from one side: the graph
    /// holds `camille --soeur de--> theo`, so reading Theo's outgoing edges
    /// never finds Camille. The edge exists, it simply leaves the other node.
    ///
    /// The scaffolding filter is the incoming mirror of
    /// [`Self::outgoing_entity_relations`]'s: `about` edges are dropped (they
    /// are the fact → hub half of the `about`/`mentions` pair), and `mentions`
    /// with them for symmetry.
    fn incoming_entity_relations(&self, id: u64) -> Result<(Vec<EntityRelation>, bool), MemoryError>
    where
        S: GraphStore,
    {
        let scanned = self
            .store
            .incoming_relations_bounded(id, crate::limits::MAX_ENTITY_SCAN_EDGES)?;
        self.resolve_entity_relations(scanned, |edge| edge.from)
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
        edges: &mut HashSet<(u64, u64, String)>,
    ) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
        for relation in relations {
            if validate_relation(&relation.predicate).is_err() {
                continue;
            }
            let subject_id = self.entity_hub(&relation.subject, entity_ids)?;
            let object_id = self.entity_hub(&relation.object, entity_ids)?;
            if subject_id == object_id {
                continue;
            }
            self.add_edge(subject_id, object_id, &relation.predicate, edges)?;
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
        edges: &mut HashSet<(u64, u64, String)>,
    ) -> Result<RememberedExtraction, MemoryError>
    where
        S: GraphStore,
    {
        let mut ids = Vec::with_capacity(facts.len());
        let mut skipped_over_cap = 0;
        for fact in facts {
            let content = fact.text.trim();
            if content.is_empty() {
                continue;
            }
            // An over-cap fact is skipped, not fatal: aborting here used to
            // leave the previous iterations persisted with no rollback, no
            // graph wiring, and no ids returned — the worst of every world.
            // Every OTHER error still aborts: they signal bad caller input
            // (reserved keys, oversized metadata) or a failing store, where
            // carrying on would compound the damage.
            let fact_id = match self.remember_inner(content, &[], metadata, None, false) {
                Ok(id) => id,
                Err(MemoryError::FactTooLarge { .. }) => {
                    skipped_over_cap += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            ids.push(fact_id);
            self.wire_entities(fact_id, &fact.entities, entity_ids, edges)?;
        }
        Ok(RememberedExtraction {
            ids,
            skipped_over_cap,
        })
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
        edges: &mut HashSet<(u64, u64, String)>,
    ) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
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
        edges: &mut HashSet<(u64, u64, String)>,
    ) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
        let entity_id = self.entity_hub(entity, entity_ids)?;
        if entity_id == fact_id {
            return Ok(());
        }
        self.add_edge(fact_id, entity_id, ABOUT_RELATION, edges)?;
        self.add_edge(entity_id, fact_id, MENTIONS_RELATION, edges)?;
        Ok(())
    }

    /// Create the edge `from -> to` labelled `label`, unless `edges` already
    /// records that triple for this call (in-call dedup only). `relate`
    /// derives the edge id from `(from, relation, to)`
    /// ([`crate::wire::hash_edge_id`] upstream in core) and is itself an O(1)
    /// idempotent no-op against an already-persisted edge, so there is
    /// nothing left to preload from the store — a prior preload here made
    /// every write to a hub with `k` existing edges cost O(k), turning `n`
    /// writes to the same hub into O(n²).
    fn add_edge(
        &self,
        from: u64,
        to: u64,
        label: &str,
        edges: &mut HashSet<(u64, u64, String)>,
    ) -> Result<(), MemoryError>
    where
        S: GraphStore,
    {
        if edges.insert((from, to, label.to_string())) {
            self.relate_inner(from, to, label)?;
        }
        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<E, S> MemoryService<E, S>
where
    E: Embedder + Send + Sync + 'static,
    S: FactStore + Send + Sync + 'static,
{
    /// The autograph worker's whole life, run on the spawned thread: ends
    /// when every sender is gone — i.e. when the handle's drop takes the
    /// sender back out of the service. Once the closing latch is up,
    /// still-queued jobs are SKIPPED: the exit pays for the job in flight,
    /// never for the queue.
    fn autograph_worker_loop(&self, rx: &std::sync::mpsc::Receiver<AutographJob>)
    where
        S: GraphStore,
    {
        let mut skipped_on_close: u64 = 0;
        for job in rx {
            if self
                .autograph_queue
                .closing
                .load(std::sync::atomic::Ordering::Acquire)
            {
                skipped_on_close += 1;
                continue;
            }
            let _generation = self.enter_generation();
            self.autograph(job.fact_id, &job.fact);
        }
        if skipped_on_close > 0 {
            self.autograph_queue
                .dropped
                .fetch_add(skipped_on_close, std::sync::atomic::Ordering::Relaxed);
            // ONE aggregated line, not one per job (#1834's rule).
            #[cfg(feature = "mcp")]
            tracing::warn!(
                skipped = skipped_on_close,
                "autograph worker closing: queued enrichments skipped — \
                 the facts are stored, their graph structure is not; \
                 re-remembering rebuilds it"
            );
        }
    }

    /// Move autograph off the response path: spawn ONE background worker
    /// consuming a bounded queue, so `remember` returns as soon as the fact
    /// is durably stored and the graph is wired behind (#1846).
    ///
    /// Measured motivation: with the production extractor, an inline
    /// autograph held every `remember` for 46-52 s while the embedding cost
    /// 0.12 s — and the MCP client timed out mid-generation, making a stored
    /// fact indistinguishable from a lost one (#1839).
    ///
    /// The read-after-write contract changes, deliberately and visibly: an
    /// `entity()` issued right after `remember` may not see the new edges
    /// yet. The fact itself is always readable immediately — only the
    /// DERIVED structure lags by one generation.
    ///
    /// One worker on purpose: the store is single-writer, and a second
    /// in-flight generation would only add contention, not throughput.
    /// `capacity` bounds the queue ([`crate::limits::MAX_AUTOGRAPH_QUEUE`]
    /// is the daemon's choice); a full queue DROPS new enrichments, counted
    /// by [`Self::autograph_dropped`] and logged — never silent, never
    /// blocking the write path.
    ///
    /// # Errors
    /// Returns [`MemoryError::Extract`] when a worker is already spawned for
    /// this service — two workers would race the single-writer store for no
    /// gain — or when the OS refuses the thread.
    pub fn spawn_autograph_worker(
        self: &std::sync::Arc<Self>,
        capacity: usize,
    ) -> Result<AutographWorkerHandle, MemoryError>
    where
        S: GraphStore,
    {
        let (tx, rx) = std::sync::mpsc::sync_channel::<AutographJob>(capacity);
        {
            let mut guard = self.autograph_queue.tx.lock();
            if guard.is_some() {
                return Err(MemoryError::Extract(crate::extract::ExtractError::Backend(
                    "autograph worker already spawned for this service".to_owned(),
                )));
            }
            *guard = Some(tx);
            // Re-arm the shutdown latch under the same lock that installs
            // the sender: a previous worker's close must not poison this one
            // into skipping every job it will ever receive.
            self.autograph_queue
                .closing
                .store(false, std::sync::atomic::Ordering::Release);
        }
        let worker_service = std::sync::Arc::clone(self);
        let join = std::thread::Builder::new()
            .name("velesdb-autograph".to_owned())
            .spawn(move || worker_service.autograph_worker_loop(&rx))
            .map_err(|err| {
                MemoryError::Extract(crate::extract::ExtractError::Backend(format!(
                    "spawn autograph worker: {err}"
                )))
            })?;
        let closer_service = std::sync::Arc::clone(self);
        Ok(AutographWorkerHandle {
            close_queue: Some(Box::new(move || {
                // Latch FIRST, sender out second: the worker observes the
                // latch no later than the queue's end, so it cannot start
                // draining jobs the shutdown meant to skip.
                closer_service
                    .autograph_queue
                    .closing
                    .store(true, std::sync::atomic::Ordering::Release);
                closer_service.autograph_queue.tx.lock().take();
            })),
            join: Some(join),
        })
    }
}
