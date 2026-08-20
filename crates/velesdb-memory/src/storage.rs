//! Storage backend abstraction for [`crate::service::MemoryService`].
//!
//! The wedge orchestration (remember/recall/relate/forget/why/fusion) is
//! written once, generic over [`MemoryStore`], so it runs unchanged over any
//! backend: the native, file-backed [`NativeStore`] (the default — nothing
//! changes for existing callers), or an in-memory backend such as the one
//! `velesdb-wasm` provides for the browser (no filesystem, no `persistence`
//! feature).

#[cfg(feature = "persistence")]
use std::collections::HashMap;
#[cfg(feature = "persistence")]
use std::path::Path;
#[cfg(feature = "persistence")]
use std::sync::Arc;

#[cfg(feature = "persistence")]
use serde_json::json;
use serde_json::Value;
#[cfg(feature = "persistence")]
use velesdb_core::agent::AgentMemory;
#[cfg(feature = "persistence")]
use velesdb_core::{Database, SearchResult};

use crate::error::MemoryError;
use crate::model::{BoundedMemoryEdges, ColumnFilter, MemoryEdge, Recollection};
#[cfg(feature = "persistence")]
use crate::mutation::{DirtyKey, MutationCapture, MutationObserver};
use crate::service::Metadata;

#[cfg(feature = "persistence")]
mod migration;

/// Fact storage: write, by-id lookup, deletion, corpus size — the core
/// facet every backend must provide. The other facets ([`RecallStore`],
/// [`GraphStore`], [`ColumnStore`]) build on stored facts; a partial
/// backend, or a test double, implements only the facets it serves and
/// the compiler refuses calls to the rest (#1959).
pub trait FactStore {
    /// Store a fact with no metadata or expiry.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError>;

    /// Store a fact tagged with `metadata`, no expiry.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError>;

    /// Store a fact that expires after `ttl_seconds`, no metadata.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError>;

    /// Store a fact with BOTH metadata and a durable TTL, in ONE write.
    ///
    /// Default: the historical two-call sequence, so a backend written before
    /// this method keeps compiling and behaving as it did. Backends that can
    /// write both at once should override it — the two-call form leaves the
    /// fact live and expiring between the calls, so a short TTL can lapse in
    /// the gap and the metadata write then fails on a fact that was perfectly
    /// valid when the caller asked for it.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if persistence fails.
    fn store_with_metadata_and_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.store_with_ttl(id, content, embedding, ttl_seconds)?;
        self.update_metadata(id, metadata)
    }

    /// Merge `metadata` into an already-stored fact's payload, preserving any
    /// durable TTL. Used to combine metadata with an expiry (store both in
    /// two calls rather than needing every metadata×TTL combination as a
    /// separate primitive).
    ///
    /// # Errors
    /// Returns [`MemoryError`] if `id` is unknown or persistence fails.
    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError>;

    /// A fact's content and embedding, or `None` if unknown/expired.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError>;

    /// A fact's raw stored payload — reserved system keys (`_veles_*`)
    /// included, so the service layer can check the hub flag before
    /// stripping them for the caller — or `None` when the fact is
    /// unknown/expired.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError>;

    /// Batched [`Self::get_metadata`]: one storage round trip for every id
    /// in `ids`, results in the same order and length (an unknown or expired
    /// id maps to `None`). Same raw-payload semantics as the single-id form.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError>;

    /// Delete a fact.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if deletion fails.
    fn delete(&self, id: u64) -> Result<(), MemoryError>;

    /// The total number of live (non-expired) tracked facts, including
    /// internal entity hubs — used as a corpus-size proxy for idf weighting.
    fn count(&self) -> usize;

    /// One cursor page of the store's live facts, ids ascending: up to
    /// `limit` entries strictly after `cursor` (`None` starts the walk),
    /// plus the cursor for the next page (`None` ends it). Payloads come
    /// back RAW — reserved keys and scaffolding markers included — because
    /// the policy of what a caller may see (hub filtering, key stripping)
    /// belongs to the service layer, in one place, for every backend.
    ///
    /// TTL-expired facts are skipped, not listed: an audit must show what
    /// the store will still serve, and a fact past its expiry is not it.
    ///
    /// Defaulted to a refusal rather than required, same reasoning as
    /// [`GraphStore::edge_count`]: an out-of-crate backend keeps compiling,
    /// and its `list_memories` answers with this error instead of a wrong
    /// walk.
    ///
    /// # Errors
    /// Returns [`MemoryError::Unsupported`] if the backend cannot enumerate
    /// at all, or [`MemoryError`] if the walk fails.
    fn list(
        &self,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<RawListedFact>, Option<u64>), MemoryError> {
        let _ = (cursor, limit);
        Err(MemoryError::Unsupported(
            "this storage backend does not support listing",
        ))
    }
}

/// Vector recall over stored facts — the surface every
/// [`crate::service::MemoryService::recall`]/`search` call goes through.
/// [`FactStore`] is a supertrait because these queries return the content
/// of the facts they rank; a backend cannot rank what it cannot store.
pub trait RecallStore: FactStore {
    /// Vector search for up to `k` ids, narrowed to facts whose metadata
    /// exactly matches every key in `filter`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the query fails.
    fn query_filtered(
        &self,
        embedding: &[f32],
        k: usize,
        filter: &Metadata,
        offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError>;

    /// Vector search for up to `k` ids, dropping facts whose metadata matches
    /// every key in `exclude`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the query fails.
    fn query_excluding(
        &self,
        embedding: &[f32],
        k: usize,
        exclude: &Metadata,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError>;
}

/// Structured columnar predicates fused with vector recall — one method
/// today, but the facet where field enumeration and richer predicates will
/// land ([`ColumnFilter`]'s op set is already `non_exhaustive`).
pub trait ColumnStore {
    /// Vector search fused with structured columnar predicates (ranges
    /// and comparisons, not just equality) — the engine behind
    /// [`crate::service::MemoryService::recall_where`].
    ///
    /// # Absent and null fields
    ///
    /// **A filter is satisfied only by a fact that HAS the field with a
    /// non-null value.** A fact missing the field, or storing `null` in it, is
    /// never returned — and `ne` is no exception, exactly as a SQL comparison
    /// against `NULL` is never true.
    ///
    /// | field state | `field != target` | `field == target` | `<` `<=` `>` `>=` |
    /// |---|---|---|---|
    /// | absent | no match | no match | no match |
    /// | present, `null` | no match | no match | no match |
    /// | present, equal | no match | match | per the comparison |
    /// | present, different | **match** | no match | per the comparison |
    ///
    /// This is stated because it did not hold: `ne` on an absent field matched
    /// on the native backend and never matched on WASM, for the API's whole
    /// life, because nothing compared them (#1759). Every backend is now held
    /// to one shared table —
    /// [`crate::column_filter_conformance`] — run against both.
    ///
    /// Null-ness is not expressible through [`ColumnFilter`]; querying for it
    /// is what `IsNull`/`IsNotNull` are for at the `VelesQL` layer.
    ///
    /// # Errors
    /// Returns [`MemoryError::InvalidFilter`] if a filter field is not a
    /// plain identifier or a filter value is non-scalar, or [`MemoryError`]
    /// if the query fails.
    fn query_columnar(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError>;
}

/// Typed graph edges between facts — the facet behind `relate`/`why` and
/// the hub walks. A backend without a graph simply does not implement it,
/// and the service methods that need it stop existing for that backend at
/// compile time.
pub trait GraphStore {
    /// Create a typed edge `from -> to`. Returns the edge id.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if either endpoint is missing or persistence fails.
    fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError>;

    /// The outgoing edges of `id`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError>;

    /// The incoming edges of `id` — the mirror of [`Self::relations`], with
    /// the same liveness rule applied to the far end (here the *source*).
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn incoming_relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError>;

    /// At most `cap` outgoing edges of `id`, plus whether its total degree
    /// exceeded the scan — the bounded twin of [`Self::relations`] (#1820).
    ///
    /// The contract is on COST, not just shape: an implementation must keep
    /// work and transient allocation O(cap), never O(degree) — a super-node
    /// (an entity hub mentioned by thousands of facts) is exactly where this
    /// accessor is reached for. `truncated` is a separate signal because
    /// `edges.len() == cap` cannot carry it: a node with exactly `cap` edges
    /// is indistinguishable from a truncated one.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn relations_bounded(&self, id: u64, cap: usize) -> Result<BoundedMemoryEdges, MemoryError>;

    /// At most `cap` incoming edges of `id`, plus whether its total incoming
    /// degree exceeded the scan — the mirror of [`Self::relations_bounded`],
    /// same O(cap) cost contract.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn incoming_relations_bounded(
        &self,
        id: u64,
        cap: usize,
    ) -> Result<BoundedMemoryEdges, MemoryError>;

    /// Remove the edge with `edge_id`. Returns `true` when it existed —
    /// idempotent: removing an absent edge is `Ok(false)`, never an error.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn unrelate(&self, edge_id: u64) -> Result<bool, MemoryError>;

    /// Remove an edge while preserving its known source for mutation capture.
    ///
    /// The default keeps third-party backends source-compatible. Native
    /// online migration overrides it so `OutgoingEdges(from)` is recorded
    /// before the edge is removed.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if storage access fails.
    fn unrelate_from(&self, from: u64, edge_id: u64) -> Result<bool, MemoryError> {
        let _ = from;
        self.unrelate(edge_id)
    }

    /// The total number of graph edges, when the backend can answer without
    /// materializing them — the observable difference between a store whose
    /// `why()` can walk somewhere and one where it degrades to plain
    /// similarity search.
    ///
    /// Defaulted to `None` ("cannot say") rather than required, deliberately:
    /// a backend outside this crate (velesdb-wasm's in-memory store) must
    /// keep compiling when this surface grows, and a wrong-but-cheap answer
    /// here would flag healthy graphs as flat. `memory_status` reports the
    /// distinction to the caller instead of papering over it.
    fn edge_count(&self) -> Option<usize> {
        None
    }
}

/// The full storage surface [`crate::service::MemoryService`] historically
/// required: every facet at once. Kept as a supertrait alias so existing
/// callers and bounds (`S: MemoryStore`) compile unchanged; the blanket
/// impl makes it automatic for any backend that implements the facets, so
/// there is nothing extra to implement and nothing to forget.
///
/// Implementors migrating from the pre-facet monolith (≤ 0.13): the
/// methods did not change — they moved. `impl MemoryStore` becomes
/// `impl FactStore + RecallStore + GraphStore + ColumnStore` (#1959).
///
/// Not to be confused with `velesdb-node`'s napi **class** `MemoryStore`,
/// which is unrelated: that is the JS-facing service object (a
/// `MemoryService` wrapper), not this storage trait. Same name, opposite
/// sides of the binding seam (#2018).
///
/// Since the facet split shipped, no bound in this workspace names the
/// alias (`S: GraphStore`-style facet bounds everywhere) — it exists only
/// so pre-facet out-of-tree code keeps compiling, and it goes away in the
/// next breaking cycle.
#[deprecated(
    since = "0.14.2",
    note = "bound on the facet traits you use (FactStore / RecallStore / \
            GraphStore / ColumnStore); MemoryStore is a compat alias since \
            the 0.14 facet split and will be removed in 0.15"
)]
pub trait MemoryStore: RecallStore + GraphStore + ColumnStore {}

// The blanket impl IS the compat shim the deprecation announces — it must
// keep compiling until the alias is removed with it in 0.15.
#[allow(deprecated)]
impl<T: RecallStore + GraphStore + ColumnStore> MemoryStore for T {}

/// One fact as [`FactStore::list`] hands it to the service layer: content
/// split out, everything else — reserved keys and scaffolding markers
/// included — still in `payload` so the service can apply its visibility
/// policy exactly once for every backend.
#[derive(Debug, Clone)]
pub struct RawListedFact {
    /// Stable id of the fact.
    pub id: u64,
    /// The stored fact text (the payload's `content` key).
    pub content: String,
    /// The rest of the stored payload, verbatim.
    pub payload: Metadata,
}

#[cfg(feature = "persistence")]
impl RawListedFact {
    /// The one place a stored payload is split into content + the rest —
    /// shared by [`MemoryStore::list`] and the JSONL export so the two
    /// reading surfaces can never disagree on what a fact's content IS.
    pub(crate) fn from_raw(fact: &crate::migration::RawFact) -> Self {
        let mut payload: Metadata = serde_json::from_str(&fact.payload).unwrap_or_default();
        let content = match payload.remove("content") {
            Some(Value::String(text)) => text,
            _ => String::new(),
        };
        Self {
            id: fact.id,
            content,
            payload,
        }
    }
}

/// The default [`MemoryStore`]: the native, file-backed engine
/// (`velesdb-core`'s `Database`/`AgentMemory`, requiring the `persistence`
/// feature). Existing callers of `MemoryService::open` see no change — this
/// is exactly what they already ran.
#[cfg(feature = "persistence")]
pub struct NativeStore {
    memory: AgentMemory,
    /// Kept beside `memory` (which owns its own clone) for the read paths
    /// that speak to the engine directly — [`MemoryStore::list`] walks the
    /// collection cursor, which `AgentMemory` does not re-expose.
    db: Arc<Database>,
    capture: MutationCapture,
}

#[cfg(feature = "persistence")]
impl NativeStore {
    /// Open (or create) a native store at `path`, sized for `dimension`.
    ///
    /// # Errors
    /// Returns [`MemoryError`] if the store cannot be opened.
    pub fn open<P: AsRef<Path>>(path: P, dimension: usize) -> Result<Self, MemoryError> {
        let db = Arc::new(Database::open(path)?);
        let memory = AgentMemory::with_dimension(Arc::clone(&db), dimension)?;
        Ok(Self {
            memory,
            db,
            capture: MutationCapture::default(),
        })
    }

    pub(crate) fn set_mutation_observer(
        &self,
        observer: Option<Arc<dyn MutationObserver>>,
    ) -> Result<(), MemoryError> {
        self.capture.replace(observer)
    }

    pub(crate) fn mutation_capture_active(&self) -> bool {
        self.capture.is_active()
    }

    fn unrelate_unobserved(&self, edge_id: u64) -> Result<bool, MemoryError> {
        self.memory
            .semantic()
            .unrelate(edge_id)
            .map_err(MemoryError::from)
    }
}

#[cfg(feature = "persistence")]
impl FactStore for NativeStore {
    fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), MemoryError> {
        self.capture.observe(DirtyKey::Fact(id))?;
        self.memory
            .semantic()
            .store(id, content, embedding)
            .map_err(MemoryError::from)
    }

    fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
    ) -> Result<(), MemoryError> {
        self.capture.observe(DirtyKey::Fact(id))?;
        self.memory
            .semantic()
            .store_with_metadata(id, content, embedding, metadata)
            .map_err(MemoryError::from)
    }

    fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.capture.observe(DirtyKey::Fact(id))?;
        self.memory
            .semantic()
            .store_with_ttl(id, content, embedding, ttl_seconds)
            .map_err(MemoryError::from)
    }

    fn update_metadata(&self, id: u64, metadata: &Metadata) -> Result<(), MemoryError> {
        self.capture.observe(DirtyKey::Fact(id))?;
        self.memory
            .semantic()
            .update_metadata(id, metadata)
            .map_err(MemoryError::from)
    }

    fn store_with_metadata_and_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Metadata,
        ttl_seconds: u64,
    ) -> Result<(), MemoryError> {
        self.capture.observe(DirtyKey::Fact(id))?;
        // Ordre delibere : le fait est ecrit avec sa metadata et SANS
        // expiration, donc il ne peut pas expirer entre les deux appels.
        // L'expiration est posee ensuite. C'est l'inverse de la sequence
        // historique (store_with_ttl puis update_metadata), ou le fait etait
        // deja vivant et deja en train d'expirer pendant la seconde ecriture.
        self.memory
            .semantic()
            .store_with_metadata(id, content, embedding, metadata)
            .map_err(MemoryError::from)?;
        self.memory
            .semantic()
            .set_ttl_durable(id, ttl_seconds)
            .map_err(MemoryError::from)
    }

    fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, MemoryError> {
        self.memory.semantic().get(id).map_err(MemoryError::from)
    }

    fn get_metadata(&self, id: u64) -> Result<Option<Metadata>, MemoryError> {
        self.memory
            .semantic()
            .get_metadata(id)
            .map_err(MemoryError::from)
    }

    fn get_metadata_batch(&self, ids: &[u64]) -> Result<Vec<Option<Metadata>>, MemoryError> {
        self.memory
            .semantic()
            .get_metadata_batch(ids)
            .map_err(MemoryError::from)
    }

    fn delete(&self, id: u64) -> Result<(), MemoryError> {
        self.capture.observe(DirtyKey::Fact(id))?;
        self.memory.semantic().delete(id).map_err(MemoryError::from)
    }

    fn count(&self) -> usize {
        self.memory.semantic().count()
    }

    fn list(
        &self,
        cursor: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<RawListedFact>, Option<u64>), MemoryError> {
        // The migration module's cursor walk, reused verbatim: id-keyed,
        // ascending, exclusive, and it skips TTL-expired points — exactly
        // the audit contract (#1762 built it to enumerate a store with full
        // fidelity, which is what an audit is).
        let (facts, next) = crate::migration::scroll_page(
            &self.db,
            self.memory.semantic().collection_name(),
            cursor,
            limit,
        )?;
        let listed = facts.iter().map(RawListedFact::from_raw).collect();
        Ok((listed, next))
    }
}

#[cfg(feature = "persistence")]
impl RecallStore for NativeStore {
    fn query_filtered(
        &self,
        embedding: &[f32],
        k: usize,
        filter: &Metadata,
        offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.memory
            .semantic()
            .query_filtered(embedding, k, filter, offset)
            .map_err(MemoryError::from)
    }

    fn query_excluding(
        &self,
        embedding: &[f32],
        k: usize,
        exclude: &Metadata,
    ) -> Result<Vec<(u64, f32, String)>, MemoryError> {
        self.memory
            .semantic()
            .query_excluding(embedding, k, exclude)
            .map_err(MemoryError::from)
    }
}

#[cfg(feature = "persistence")]
impl ColumnStore for NativeStore {
    fn query_columnar(
        &self,
        embedding: &[f32],
        k: usize,
        filters: &[ColumnFilter],
    ) -> Result<Vec<Recollection>, MemoryError> {
        let (sql, params) = self.build_fused_query(embedding, k, filters)?;
        // Field names are validated by `build_fused_query`; ensure each one is
        // indexed so the planner uses a bitmap prefilter instead of an O(n)
        // post-filter scan. Idempotent and incrementally maintained thereafter.
        for field in filters
            .iter()
            .map(|filter| filter.field.as_str())
            .chain(INTERNAL_MARKER_FIELDS.iter().copied())
        {
            self.memory
                .semantic()
                .ensure_index(field)
                .map_err(MemoryError::from)?;
        }
        let results = self
            .memory
            .query_semantic(&sql, &params)
            .map_err(MemoryError::from)?;
        Ok(results.iter().map(to_recollection).collect())
    }
}

#[cfg(feature = "persistence")]
impl GraphStore for NativeStore {
    fn relate(&self, from: u64, to: u64, relation: &str) -> Result<u64, MemoryError> {
        self.capture.observe(DirtyKey::OutgoingEdges(from))?;
        self.memory
            .semantic()
            .relate(from, to, relation, None)
            .map_err(MemoryError::from)
    }

    fn relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        Ok(to_memory_edges(self.memory.semantic().relations(id)?))
    }

    fn incoming_relations(&self, id: u64) -> Result<Vec<MemoryEdge>, MemoryError> {
        Ok(to_memory_edges(
            self.memory.semantic().incoming_relations(id)?,
        ))
    }

    fn relations_bounded(&self, id: u64, cap: usize) -> Result<BoundedMemoryEdges, MemoryError> {
        let bounded = self.memory.semantic().relations_bounded(id, cap)?;
        Ok(BoundedMemoryEdges {
            edges: to_memory_edges(bounded.edges),
            truncated: bounded.truncated,
        })
    }

    fn incoming_relations_bounded(
        &self,
        id: u64,
        cap: usize,
    ) -> Result<BoundedMemoryEdges, MemoryError> {
        let bounded = self.memory.semantic().incoming_relations_bounded(id, cap)?;
        Ok(BoundedMemoryEdges {
            edges: to_memory_edges(bounded.edges),
            truncated: bounded.truncated,
        })
    }

    fn unrelate(&self, edge_id: u64) -> Result<bool, MemoryError> {
        if self.capture.is_active() {
            return Err(MemoryError::MigrationCapture(format!(
                "cannot remove edge {edge_id} without its source id"
            )));
        }
        self.unrelate_unobserved(edge_id)
    }

    fn unrelate_from(&self, from: u64, edge_id: u64) -> Result<bool, MemoryError> {
        self.capture.observe(DirtyKey::OutgoingEdges(from))?;
        self.unrelate_unobserved(edge_id)
    }

    fn edge_count(&self) -> Option<usize> {
        // A collection-access failure here means the store is unusable for
        // every other call too; for a status readout "cannot say" is the
        // honest degradation, not an error path of its own.
        self.memory.semantic().edge_count().ok()
    }
}

/// Map core [`GraphEdge`](velesdb_core::collection::graph::GraphEdge)s to the
/// wire-facing [`MemoryEdge`] shape — shared by both edge directions, so the
/// two can never disagree on which endpoint or id they report.
#[cfg(feature = "persistence")]
fn to_memory_edges(edges: Vec<velesdb_core::collection::graph::GraphEdge>) -> Vec<MemoryEdge> {
    edges
        .into_iter()
        .map(|edge| MemoryEdge {
            id: edge.id(),
            from: edge.source(),
            to: edge.target(),
            relation: edge.label().to_owned(),
        })
        .collect()
}

#[cfg(feature = "persistence")]
impl NativeStore {
    /// Build the `VelesQL` for [`Self::query_columnar`]: a `NEAR` predicate
    /// plus one bound parameter per filter, against the semantic collection.
    /// Filter *values* are bound as query parameters (never interpolated);
    /// filter *field names* are validated to be plain identifiers.
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
            validate_column_filter(filter)?;
            let key = format!("p{index}");
            // `ne` is spelled out rather than left to `!=` alone. Core's
            // `Condition::Neq` is `is_none_or`, so a bare `field != $p` also
            // matches a fact that HAS no such field — which made `ne` mean two
            // different things on the two backends (#1759). Requiring the
            // field first pins the published contract here, at the adapter,
            // instead of redefining `!=` for all of `VelesQL` — that is a
            // product-wide semantic break, deliberately out of scope.
            //
            // `IS NOT NULL` is false for an absent field AND for an explicit
            // `null`, which is exactly the contract: a comparison against null
            // is never true, as in SQL. `IsNull`/`IsNotNull` stay the operators
            // dedicated to null-ness.
            if matches!(filter.op, crate::model::ColumnOp::Ne) {
                let _ = write!(predicate, " AND {} IS NOT NULL", filter.field);
            }
            let _ = write!(
                predicate,
                " AND {} {} ${key}",
                filter.field,
                filter.op.as_sql()
            );
            params.insert(key, filter.value.clone());
        }
        // Exclude internal scaffolding INSIDE the query rather than after it:
        // the engine applies `LIMIT k`, so a post-filter would quietly return
        // fewer than `k` caller facts whenever artefacts crowd the ranking.
        //
        // `!=` is what excludes here, and it works for the same reason the
        // leak existed: `Condition::Neq` is `is_none_or`, so a fact that has
        // no such column at all MATCHES. Applied to a marker, that keeps every
        // caller fact (none carries one) and drops exactly the class that
        // does. These names are compile-time constants, never caller input,
        // so they go straight into the text without passing through
        // `validate_column_filter` — whose job is to reject a CALLER filter
        // naming a reserved key.
        //
        // The asymmetry with the caller loop above is DELIBERATE and load-
        // bearing: a caller's `ne` now carries an `IS NOT NULL`, this exclusion
        // must NOT. It depends on the absent field matching — that is how it
        // keeps every caller fact while dropping the marked ones. Giving these
        // markers the same treatment would exclude every caller fact instead,
        // since none of them carries a marker column at all.
        for (index, marker) in INTERNAL_MARKER_FIELDS.iter().enumerate() {
            let key = format!("m{index}");
            let _ = write!(predicate, " AND {marker} != ${key}");
            params.insert(key, json!(true));
        }
        let sql = format!(
            "SELECT * FROM {} WHERE {predicate} LIMIT {k}",
            self.memory.semantic().collection_name()
        );
        Ok((sql, params))
    }
}

/// Reserved metadata key `remember`/`remember_with_ttl` auto-stamp with
/// today's date (a `YYYYMMDD` integer, [`crate::clock::today_ymd`]) whenever
/// the caller didn't already set it — see
/// [`crate::service::MemoryService::remember_with_ttl`] for the full
/// contract. A deliberate, documented **exception** to every other
/// `_veles_`-namespaced key: [`is_reserved_key`] still names it (so it can
/// never be confused with an arbitrary caller field), but unlike a true
/// system key —
/// - a caller MAY set it explicitly (to date a fact retroactively; never
///   overwritten once present), and
/// - it is NOT stripped from caller-facing results, so
///   [`crate::dated_context::format_dated_context`]'s `date_field` (wired
///   through `recall_fused`'s `date_field` parameter) can read it back with
///   zero caller effort.
///
/// `pub` (re-exported at the crate root) so every caller of `date_field`
/// names this one string in exactly one place, not a copy-pasted literal.
pub const AUTO_DATE_FIELD: &str = "_veles_date";

/// True for metadata keys the memory layer reserves: the engine's `content`
/// payload, and any `_veles_`-namespaced system key (durable TTL, entity
/// hubs) — [`AUTO_DATE_FIELD`] EXCEPTED, since (unlike every other reserved
/// key) it is caller-settable and caller-visible by design. The single
/// source of the reserved-key contract — the service layer (reject/strip)
/// and every backend enforce it through this one predicate.
pub(crate) fn is_reserved_key(key: &str) -> bool {
    key != AUTO_DATE_FIELD && (key == "content" || key.starts_with("_veles_"))
}

/// Marks an entity hub minted by `remember_extracted` — graph scaffolding,
/// never a fact the caller stored.
pub const HUB_FIELD: &str = "_veles_hub";
/// Marks a compilation event recorded for `context_savings`.
pub const CTX_EVENT_FIELD: &str = "_veles_ctx_event";
/// Marks a stored compilation source, served back by `retrieve_context_source`.
pub const CTX_SOURCE_FIELD: &str = "_veles_ctx_source";
/// Marks a saved working context, served back by `load_working_context`.
pub const CTX_WORKING_FIELD: &str = "_veles_ctx_working";
/// Marks a project's working-context index, read by `list_working_contexts`.
pub const CTX_WORKING_INDEX_FIELD: &str = "_veles_ctx_working_index";

/// Every marker that identifies a stored fact as internal scaffolding rather
/// than a caller memory. Facts of these five classes live in the same
/// collection as caller facts and are written by exactly one path each; the
/// markers are declared here, and imported by those paths, so the write and
/// the exclusion cannot drift apart.
///
/// The discriminant is the PRESENCE of one of these keys — deliberately NOT
/// the `_veles_` prefix. [`AUTO_DATE_FIELD`] (`_veles_date`) is reserved too
/// and is stamped onto ordinary CALLER facts, so a prefix test would hide the
/// entire store instead of the scaffolding.
pub const INTERNAL_MARKER_FIELDS: &[&str] = &[
    HUB_FIELD,
    CTX_EVENT_FIELD,
    CTX_SOURCE_FIELD,
    CTX_WORKING_FIELD,
    CTX_WORKING_INDEX_FIELD,
];

/// Whether a raw payload belongs to one of the five internal classes.
///
/// `pub` and shared for the same reason as [`validate_column_filter`]: a
/// caller-facing recall path must not depend on which backend answered it.
/// A backend that can test the payload directly should use this; one that
/// pushes the predicate into a query builds the equivalent there — the
/// authority on *which* markers count is this list either way.
#[must_use]
pub fn is_internal_scaffolding(payload: &Metadata) -> bool {
    INTERNAL_MARKER_FIELDS
        .iter()
        .any(|marker| payload.contains_key(*marker))
}

/// Drop reserved system keys from a raw payload, and collapse an
/// empty-after-stripping map to `None` — the caller-facing shape every
/// [`Recollection::metadata`] is built from. `pub` because a [`MemoryStore`]
/// backend that assembles `Recollection`s itself (`query_columnar`) must
/// apply the same stripping the service layer applies on every other recall
/// path, or reserved keys leak to callers on that one path only.
#[must_use]
pub fn strip_reserved_keys(payload: Option<Metadata>) -> Option<Metadata> {
    payload.and_then(|payload| {
        let metadata: Metadata = payload
            .into_iter()
            .filter(|(key, _)| !is_reserved_key(key))
            .collect();
        (!metadata.is_empty()).then_some(metadata)
    })
}

/// [`strip_reserved_keys`] over a *borrowed* payload: clones only the
/// surviving non-reserved entries. Use this when the payload isn't already
/// owned — cloning the whole map first would deep-copy the reserved
/// `content` value (the full fact text) per hit, only to discard it.
#[must_use]
pub fn strip_reserved_keys_ref(payload: Option<&Metadata>) -> Option<Metadata> {
    payload.and_then(|payload| {
        let metadata: Metadata = payload
            .iter()
            .filter(|(key, _)| !is_reserved_key(key))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        (!metadata.is_empty()).then_some(metadata)
    })
}

/// Map a core search result to a [`Recollection`], lifting the fact text out
/// of the reserved `content` payload key and surfacing any remaining
/// caller-supplied metadata (reserved system keys excluded).
#[cfg(feature = "persistence")]
fn to_recollection(result: &SearchResult) -> Recollection {
    let payload = result.point.payload.as_ref().and_then(Value::as_object);
    let content = payload
        .and_then(|payload| payload.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Recollection {
        id: result.point.id,
        score: result.score,
        content,
        metadata: strip_reserved_keys_ref(payload),
    }
}

/// Validate one `recall_where` column filter: a plain, non-reserved
/// identifier field name and a scalar (string/number/boolean) value. `pub`
/// and shared so every [`MemoryStore`] backend enforces the *same* documented
/// contract — the field-name rule keeps a filter safe to place into query
/// text (`NativeStore` builds `VelesQL`; values are always bound parameters),
/// and rejects the reserved system columns (`content`, `_veles_*`) regardless
/// of backend; the scalar rule turns what would be an opaque engine error
/// into a clear client-input error.
///
/// # Errors
/// Returns [`MemoryError::InvalidFilter`] when either rule is violated.
pub fn validate_column_filter(filter: &ColumnFilter) -> Result<(), MemoryError> {
    let field = &filter.field;
    let plain = !field.is_empty() && field.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !plain || is_reserved_key(field) {
        return Err(MemoryError::InvalidFilter(field.clone()));
    }
    match &filter.value {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(()),
        value => Err(MemoryError::InvalidFilter(format!(
            "value must be a string, number, or boolean, got {value}"
        ))),
    }
}

#[cfg(all(test, feature = "persistence"))]
#[path = "storage_tests.rs"]
mod tests;
