//! The memory service: five operations over the in-core Agent Memory SDK.
//!
//! # Role: the crate's assembler — declared, with a budget
//!
//! This module is the ONE place allowed to know every seam at once — store,
//! embedder, extractor, fusion, autograph, online migration, the working
//! context bridge — and wire them into the operations the adapters call.
//! That role is why it stays the crate's densest module (~1 000 lines after
//! the #2021 cut, against the ~500-NLOC file budget the child-module splits
//! aim at), and why its size is a *declared* cost rather than an accident:
//! an assembler that was split by size alone would scatter the wiring it
//! exists to make readable.
//!
//! The budget still binds. Growth goes into child modules that keep private
//! access (`fused_recall.rs` and `online_migration.rs` are the pattern —
//! `#[path]` children of `service`, not siblings), never into this file. The
//! working-context bridge was the last named cut and it is done: it left along
//! the store-facet line of #1959, as #1967 asked. A change that adds an
//! operation body here instead of a child module needs to say why in review.

use std::collections::{HashMap, HashSet};
#[cfg(feature = "persistence")]
use std::path::Path;
#[cfg(feature = "persistence")]
use std::sync::Arc;

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
    ColumnFilter, EntityProfile, EntityRelation, Explanation, Link, MemoryEdge, MemoryNode,
    Recollection, RememberedExtraction, UnrelateOutcome,
};
#[cfg(feature = "persistence")]
use crate::mutation::MutationObserver;
#[cfg(feature = "persistence")]
use crate::storage::NativeStore;
use crate::storage::{
    is_reserved_key, strip_reserved_keys, ColumnStore, FactStore, GraphStore, RecallStore,
    AUTO_DATE_FIELD,
};

/// [`MemoryService::recall_fused`] and its helpers — split out to keep this
/// file under the crate's 500-NLOC-per-file budget, same pattern as
/// `velesdb-core`'s `database/*.rs` split. A child module of `service`, so it
/// shares full access to `MemoryService`'s private fields and methods.
#[path = "fused_recall.rs"]
mod fused_recall;

/// The graph facet — `relate`/`forget`/hubs/`why`/`traverse` — same split,
/// same reason; see `service_graph.rs`'s module doc.
#[path = "service_graph.rs"]
mod graph;

/// The graph facet's *construction* half — `remember*`/`autograph*` and the
/// `wire_*` plumbing — same split, same reason; see its module doc (#2021).
#[path = "service_graph_wiring.rs"]
mod graph_wiring;

#[cfg(feature = "persistence")]
#[allow(dead_code)]
#[path = "online_migration.rs"]
mod online_migration;
#[cfg(feature = "persistence")]
pub(crate) use online_migration::recover_startup;
#[cfg(feature = "mcp")]
pub(crate) use online_migration::{
    JobPhase, JobTarget, LiveGenerationSlot, MigrationStartConfig, MigrationStatus,
    OnlineMigrationManager,
};

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
///
/// Re-exported from [`crate::storage`] rather than spelled out again here: it
/// is one of the five markers [`crate::storage::INTERNAL_MARKER_FIELDS`]
/// excludes from `recall_where`, and a second literal could drift from that
/// list without any test noticing.
use crate::storage::HUB_FIELD;
/// Salt mixed into a hub's stable id so the hub id space is disjoint from
/// natural fact ids: a caller fact whose text happens to equal a hub's display
/// content (`Entity: rust`) can never collide with, or overwrite, the hub.
const HUB_ID_SALT: &str = "\u{0}_veles_entity_hub\u{0}";
/// Edge label a hub uses to point back at a fact it tags (the hub → fact
/// direction). [`fused_recall`] reads this to recognise which edges in a
/// `why()` walk crossed a hub, so it can weight the reached fact by that
/// hub's specificity instead of a flat constant.
const MENTIONS_RELATION: &str = "mentions";
/// Edge label a fact uses to point at a hub it is tagged with (the fact → hub
/// direction) — [`MENTIONS_RELATION`]'s bipartite twin, written by
/// [`MemoryService::remember_extracted`]'s `wire_entity`.
const ABOUT_RELATION: &str = "about";

/// Local-first agent memory backed by a single `VelesDB` instance.
///
/// Generic over the [`Embedder`] so production can use an on-device model while
/// tests use a deterministic, network-free one, and over the [`FactStore`]
/// backend `S` so the same orchestration runs over the native, file-backed
/// engine (the default — nothing changes for existing callers) or any other
/// backend that implements the storage facets it uses (e.g. an in-memory one
/// for WASM). Methods needing recall, graph, or columnar capability carry
/// that facet as an extra bound — a partial backend simply does not have
/// those methods (#1959).
///
/// Two definitions, `persistence`-gated: the default type parameter itself
/// references [`NativeStore`], which doesn't exist as a type at all without
/// the feature, so a `persistence`-free build (e.g. `velesdb-wasm`) drops the
/// default and every caller names its own storage backend explicitly. The
/// duplication stops at the type-parameter list: the field lists are
/// identical by contract — what varies per feature is the *type* of
/// [`GenerationGate`], never the shape of the service — and
/// `tests/service_field_drift.rs` fails the build of any change that lets
/// them diverge again (#2017).
#[cfg(feature = "persistence")]
pub struct MemoryService<E: Embedder, S: FactStore = NativeStore> {
    store: S,
    embedder: E,
    autograph: Option<crate::extract::DynExtractor>,
    autograph_queue: AutographQueue,
    generation_gate: GenerationGate,
}
#[cfg(not(feature = "persistence"))]
pub struct MemoryService<E: Embedder, S: FactStore> {
    store: S,
    embedder: E,
    autograph: Option<crate::extract::DynExtractor>,
    autograph_queue: AutographQueue,
    generation_gate: GenerationGate,
}

#[path = "service_generation.rs"]
mod service_generation;
use service_generation::{GenerationGate, GenerationGuard};

/// One deferred autograph: the stored fact a background worker will read for
/// entities, edges and attributes (#1846).
// The fields are read only on the worker path, which `spawn_autograph_worker`
// cfg-gates off wasm32 (no threads there) — without this the wasm check dies
// on dead_code under -D warnings.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct AutographJob {
    fact_id: u64,
    fact: String,
}

/// The decoupling state of [`MemoryService::autograph`] (#1846).
///
/// Empty by default: every construction path starts with autograph running
/// INLINE, exactly as before — the WASM binding has no threads, library
/// consumers keep the synchronous contract, and every existing test stays
/// meaningful. [`MemoryService::spawn_autograph_worker`] fills `tx`, after
/// which `remember` only ENQUEUES: the caller stops paying the generation on
/// its response path — 46 s measured for a 12-word fact on the production
/// daemon, versus a 0.12 s embedding — and the worker wires the graph behind.
///
/// `dropped` counts enrichments refused by a FULL queue or skipped by a
/// closing worker. Non-negotiably visible: a burst that outruns the
/// extractor loses graph structure, and a loss nobody can see is the exact
/// defect class #1820 closed for responses.
///
/// `closing` is the shutdown latch: the handle's drop raises it BEFORE
/// removing the sender, so the worker finishes the job in flight and SKIPS
/// what is still queued (counted, one aggregated warning) instead of
/// draining a queue of generations — 64 × a 46 s model would hold the
/// daemon's exit for tens of minutes. Re-armed by each spawn.
#[derive(Default)]
// `closing` is read only by the worker/drop path, absent on wasm32 — same
// rationale as `AutographJob` above.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct AutographQueue {
    tx: parking_lot::Mutex<Option<std::sync::mpsc::SyncSender<AutographJob>>>,
    dropped: std::sync::atomic::AtomicU64,
    /// Enrichments that RAN and failed part-way through wiring — distinct
    /// from `dropped` (never ran). See `MemoryService::autograph_failed`.
    failed: std::sync::atomic::AtomicU64,
    closing: std::sync::atomic::AtomicBool,
}

/// Join guard for the background autograph worker.
///
/// Dropping it raises the closing latch, takes the sender out of the
/// service, and JOINS the worker: the job in flight completes, the
/// still-queued ones are SKIPPED — counted in the drop counter, one
/// aggregated warning — and only then does the drop return. Tests get
/// determinism; the daemon's shutdown waits for at most ONE generation,
/// never a queue of them.
pub struct AutographWorkerHandle {
    close_queue: Option<Box<dyn FnOnce() + Send + Sync>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for AutographWorkerHandle {
    fn drop(&mut self) {
        if let Some(close) = self.close_queue.take() {
            close();
        }
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
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
            autograph_queue: AutographQueue::default(),
            generation_gate: GenerationGate::new(),
        })
    }

    pub(crate) fn install_mutation_observer(
        &self,
        observer: Option<Arc<dyn MutationObserver>>,
    ) -> Result<(), MemoryError> {
        let _generation = self.generation_gate.write();
        self.store.set_mutation_observer(observer)
    }

    pub(crate) fn migration_capture_active(&self) -> bool {
        self.store.mutation_capture_active()
    }
}

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// Build a service directly over a `store` backend, bypassing
    /// [`Self::open`]'s filesystem-specific setup — the constructor a
    /// non-native backend (e.g. `velesdb-wasm`'s in-memory store) uses.
    pub fn with_store(store: S, embedder: E) -> Self {
        Self {
            store,
            embedder,
            autograph: None,
            autograph_queue: AutographQueue::default(),
            generation_gate: GenerationGate::new(),
        }
    }

    fn enter_generation(&self) -> GenerationGuard<'_> {
        GenerationGuard {
            _guard: self.generation_gate.read(),
        }
    }

    /// Turn on **autograph**: every [`Self::remember`] additionally reads the
    /// stored fact for entities, entity→entity edges and entity attributes,
    /// and wires them — so the knowledge graph builds itself from ordinary
    /// `remember` calls, with no separate [`Self::remember_extracted`].
    ///
    /// Opt-in, and off unless this is called. It runs in one of two modes:
    /// **inline** by default — the enrichment costs one generation per
    /// `remember`, on the caller's write path, which is a real latency and
    /// availability change: a memory write that silently depends on a local
    /// model being up is not a default anyone should inherit — or
    /// **decoupled** when [`Self::spawn_autograph_worker`] is active, where
    /// `remember` returns as soon as the fact is durably stored and the
    /// derived edges lag by one generation (an `entity`/`why` read issued
    /// immediately after may not see them yet; the fact itself is always
    /// immediately readable).
    ///
    /// The caller's fact is stored **verbatim and first**. Autograph only
    /// *adds* structure around it; it never rewrites or replaces what the
    /// caller asked to remember.
    #[must_use]
    pub fn with_autograph(mut self, extractor: crate::extract::DynExtractor) -> Self {
        self.autograph = Some(extractor);
        self
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

    /// Whether an autograph extractor is configured at all.
    #[must_use]
    pub fn has_autograph(&self) -> bool {
        self.autograph.is_some()
    }

    /// The total number of live tracked facts, internal entity hubs included
    /// — the store's [`FactStore::count`], relayed for `memory_status`.
    #[must_use]
    pub fn fact_count(&self) -> usize {
        let _generation = self.enter_generation();
        self.store.count()
    }

    /// One page of the store's facts, for auditing — "what does my agent
    /// know?" — which `recall` structurally cannot answer: it ranks by
    /// resemblance to a query, and what resembles nothing you thought to
    /// ask stays invisible.
    ///
    /// The store hands back raw pages ([`FactStore::list`]); the
    /// visibility policy is applied here, once, for every backend: internal
    /// entity hubs are skipped unless `include_internal` (they are the
    /// graph's scaffolding, not the user's facts), reserved `_veles_*` keys
    /// are stripped exactly as `recall` strips them (the auto-stamped date
    /// survives — an audit legitimately asks WHEN), and `filter` keeps only
    /// facts whose metadata equals every given key. A filtered page may
    /// come back sparse — the cursor still advances over what was skipped,
    /// so the WALK stays exhaustive.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the backend cannot enumerate or the walk
    /// fails.
    pub fn list(
        &self,
        cursor: Option<u64>,
        limit: usize,
        filter: Option<&Metadata>,
        include_internal: bool,
    ) -> Result<(Vec<crate::model::ListedMemory>, Option<u64>), MemoryError> {
        let _generation = self.enter_generation();
        let limit = crate::limits::clamp_recall_limit(limit.max(1));
        let (page, next) = self.store.list(cursor, limit)?;
        let memories = page
            .into_iter()
            .filter_map(|fact| audited(fact, filter, include_internal))
            .collect();
        Ok((memories, next))
    }
}

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
    /// Cheap "is this fact still stored?" probe gating [`Self::autograph`]'s
    /// wiring: the same `store.get` existence check [`Self::forget`] and
    /// [`Self::ensure_exists`] use. A store read error answers `false` —
    /// autograph must never fabricate structure for a fact it cannot prove is
    /// still there, and a missing (or unprovable) fact is a clean skip, never
    /// an error on this deliberately-infallible path.
    fn fact_exists(&self, fact_id: u64) -> bool {
        matches!(self.store.get(fact_id), Ok(Some(_)))
    }

    /// Extract and orient one passage without writing any memory state.
    ///
    /// Kept separate from [`Self::store_extraction`] so the MCP durable-job
    /// worker can persist the model output before the first graph write. A
    /// restart after that boundary replays stable data instead of generating a
    /// second, potentially different extraction.
    pub(crate) fn extract_passage<X: Extractor>(
        text: &str,
        extractor: &X,
    ) -> Result<crate::extract::Extraction, MemoryError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(MemoryError::EmptyFact);
        }
        let mut extraction = extractor.extract_graph(text)?;
        crate::extract::orient_kinship(text, &mut extraction.relations);
        Ok(extraction)
    }

    /// Shared resolver for both edge directions: skip the bipartite
    /// scaffolding labels, resolve each edge's far end (`far_end` picks which
    /// endpoint that is) to its stored content — at most
    /// [`crate::limits::MAX_ENTITY_RELATIONS`] of them — and say whether the
    /// result is a partial view (#1820).
    ///
    /// Truncated when either budget bit: the store's raw scan window
    /// ([`crate::limits::MAX_ENTITY_SCAN_EDGES`]) left edges unread, or a
    /// typed edge past the resolution cap was seen and dropped. Both cuts
    /// are the same honest signal — "there is more than this view shows".
    fn resolve_entity_relations(
        &self,
        scanned: crate::model::BoundedMemoryEdges,
        far_end: impl Fn(&MemoryEdge) -> u64,
    ) -> Result<(Vec<EntityRelation>, bool), MemoryError> {
        let mut relations = Vec::new();
        let mut truncated = scanned.truncated;
        for edge in scanned.edges {
            if edge.relation == MENTIONS_RELATION || edge.relation == ABOUT_RELATION {
                continue;
            }
            if relations.len() >= crate::limits::MAX_ENTITY_RELATIONS {
                truncated = true;
                break;
            }
            let far = far_end(&edge);
            let content = self.store.get(far)?.map(|(content, _)| content);
            relations.push(EntityRelation {
                predicate: edge.relation,
                target_id: far,
                target: content.unwrap_or_default(),
            });
        }
        Ok((relations, truncated))
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
    ) -> Result<Vec<Recollection>, MemoryError>
    where
        S: RecallStore,
    {
        let _generation = self.enter_generation();
        self.recall_inner(query, k, filter)
    }

    fn recall_inner(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
    ) -> Result<Vec<Recollection>, MemoryError>
    where
        S: RecallStore,
    {
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
    ) -> Result<Vec<(u64, f32, String)>, MemoryError>
    where
        S: RecallStore,
    {
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
    /// **Caller memories only.** The store also holds internal scaffolding —
    /// the entity hubs of [`Self::remember_extracted`] and the context
    /// compiler's four artefact classes (stored sources, compilation events,
    /// working contexts, and the per-project working-context index). They sit
    /// in the same collection as caller facts and are excluded from every
    /// result here, whatever the predicate.
    ///
    /// That exclusion is applied by the backend against
    /// [`crate::storage::INTERNAL_MARKER_FIELDS`]; it is NOT a consequence of
    /// those facts being unfilterable. A caller cannot write a filter naming a
    /// reserved key, but `field ne value` MATCHES a fact that has no such
    /// field at all — and scaffolding has none of the caller's columns, so
    /// before #1737 every `ne` predicate returned all of it.
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
    ) -> Result<Vec<Recollection>, MemoryError>
    where
        S: ColumnStore + RecallStore,
    {
        let _generation = self.enter_generation();
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
            return self.recall_inner(query, k, None);
        }
        let embedding = self.embedder.embed(query)?;
        self.store.query_columnar(&embedding, k, filters)
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
/// The audit's per-fact visibility policy, in one place: `None` skips the
/// fact (internal scaffolding under the default view, or a metadata filter
/// miss), `Some` carries what the caller may see — reserved keys stripped
/// exactly as recall strips them, or the raw payload under
/// `include_internal`.
fn audited(
    fact: crate::storage::RawListedFact,
    filter: Option<&Metadata>,
    include_internal: bool,
) -> Option<crate::model::ListedMemory> {
    if !include_internal && crate::storage::is_internal_scaffolding(&fact.payload) {
        return None;
    }
    let matches = filter.is_none_or(|wanted| {
        wanted
            .iter()
            .all(|(key, value)| fact.payload.get(key) == Some(value))
    });
    if !matches {
        return None;
    }
    let metadata = if include_internal {
        (!fact.payload.is_empty()).then_some(fact.payload)
    } else {
        strip_reserved_keys(Some(fact.payload))
    };
    Some(crate::model::ListedMemory {
        id: fact.id,
        content: fact.content,
        metadata,
    })
}

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
    validate_embeddable(fact)
}

/// Refuse a text past the size an embedding model still accepts.
///
/// Extracted from [`validate_fact`] so every path that embeds CALLER content
/// answers to the same cap: `remember` refuses (this function), while the
/// context bridge truncates via [`embeddable_prefix`] — but neither may hand
/// the backend an oversized text and relay its raw failure, which is how
/// issue #1654's `ollama embeddings call failed` (naming neither size nor
/// cap) reached users.
pub(crate) fn validate_embeddable(text: &str) -> Result<(), MemoryError> {
    if text.len() > crate::limits::MAX_EMBEDDABLE_TEXT_BYTES {
        return Err(MemoryError::FactTooLarge {
            bytes: text.len(),
            max: crate::limits::MAX_EMBEDDABLE_TEXT_BYTES,
        });
    }
    Ok(())
}

/// The longest prefix of `text` that fits the embeddable cap without
/// splitting a UTF-8 character.
///
/// For content that must be STORED whole but whose vector only serves
/// similarity search (context sources: retrieval is hash-addressed, the
/// vector is a ranking aid), truncating the *embedded* text is the correct
/// trade — refusing would fail a legitimate compile, and a placeholder
/// vector would remove the source from semantic recall entirely.
///
/// Gated on `context`: the compiler's source writer is its only caller, so a
/// build without that feature sees dead code, which CI's `-D warnings`
/// turns into a failed build. Same shape as `positive_ttl` — and only the
/// per-feature ISOLATION loop catches it, never a feature combination.
#[cfg(feature = "context")]
pub(crate) fn embeddable_prefix(text: &str) -> &str {
    let cap = crate::limits::MAX_EMBEDDABLE_TEXT_BYTES;
    if text.len() <= cap {
        return text;
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
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
