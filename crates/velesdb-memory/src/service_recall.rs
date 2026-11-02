//! Vector recall — [`MemoryService::recall`], [`MemoryService::recall_where`]
//! and the candidate search they share with `why`, `recall_fused` and the
//! anchored recall. A `#[path]` child of `service`, split out under the
//! file-budget rule like `fused_recall.rs`, so it keeps private access.

use serde_json::{Map, Value};

use super::{reject_reserved_keys, MemoryService, Metadata, HUB_FIELD};
use crate::embedder::Embedder;
use crate::error::MemoryError;
use crate::model::{ColumnFilter, Recollection};
use crate::storage::{strip_reserved_keys, ColumnStore, FactStore, RecallStore};

/// One vector hit: `(id, similarity, content)`.
pub(super) type Hit = (u64, f32, String);
/// Recall candidates: the hits in similarity order and their raw payloads.
pub(super) type Candidates = (Vec<Hit>, Vec<Option<Metadata>>);

impl<E: Embedder, S: FactStore> MemoryService<E, S> {
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
        let Some((hits, payloads)) = self.recall_candidates(query, k, filter)? else {
            return Ok(Vec::new());
        };
        // RL Memory: re-order the recalled set by learned confidence. Facts
        // that never received `feedback` keep their similarity order exactly.
        #[cfg(feature = "persistence")]
        let (hits, payloads) = Self::rl_rerank(hits, payloads);
        Ok(hits.into_iter().zip(payloads).map(recollection).collect())
    }

    /// The candidate set of a recall, in similarity order, with each hit's
    /// RAW payload (reserved keys included) — `None` for a blank query.
    ///
    /// One batched payload lookup serves every later concern — the RL
    /// re-rank, anchor freshness, and the caller-facing metadata — a single
    /// round trip, not one per concern.
    pub(super) fn recall_candidates(
        &self,
        query: &str,
        k: usize,
        filter: Option<&Metadata>,
    ) -> Result<Option<Candidates>, MemoryError>
    where
        S: RecallStore,
    {
        let query = query.trim();
        if query.is_empty() {
            return Ok(None);
        }
        reject_reserved_keys(filter)?;
        let embedding = self.embedder.embed(query)?;
        let hits = self.search(&embedding, k, filter)?;
        let ids: Vec<u64> = hits.iter().map(|(id, _, _)| *id).collect();
        let payloads = self.store.get_metadata_batch(&ids)?;
        Ok(Some((hits, payloads)))
    }

    /// Vector search for up to `k` ids, optionally narrowed by a metadata
    /// `filter`. Shared by [`Self::recall`] and [`Self::why`].
    pub(super) fn search(
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

/// The caller-facing shape of one hit: reserved payload keys stripped.
pub(super) fn recollection(
    ((id, score, content), payload): (Hit, Option<Metadata>),
) -> Recollection {
    Recollection {
        id,
        score,
        content,
        metadata: strip_reserved_keys(payload),
    }
}
