//! Semantic Memory - Long-term knowledge storage (US-002)
//!
//! Stores facts and knowledge as vectors with similarity search.
//! Each fact has an ID, content text, embedding vector, and optional metadata.

use crate::{Database, Point};
use parking_lot::RwLock;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::memory_helpers;
use super::ttl::{MemoryKind, MemoryTtl};

/// Long-term semantic memory for storing knowledge facts with vector similarity search.
///
/// Each fact is stored as an embedding vector with associated text content.
/// Supports TTL-based expiration and snapshot serialization.
pub struct SemanticMemory {
    collection_name: String,
    db: Arc<Database>,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    stored_ids: RwLock<HashSet<u64>>,
    /// Edge-id allocator for [`Self::relate`] (seeded past existing edges).
    next_edge_id: std::sync::atomic::AtomicU64,
}

impl SemanticMemory {
    const COLLECTION_NAME: &'static str = "_semantic_memory";

    /// Creates or opens semantic memory with an **independent** in-memory TTL.
    ///
    /// # Standalone limitation
    ///
    /// The [`MemoryTtl`] allocated here is not shared with any snapshot
    /// mechanism. TTLs assigned at store time ([`Self::store_with_ttl`]) are
    /// durable: the expiry is persisted as a `_veles_expires_at` payload field and
    /// the in-memory map is rebuilt from payloads at construction, so they
    /// survive a restart. TTLs set only in the map (e.g. via
    /// `AgentMemory::set_semantic_ttl`) remain in-memory, and
    /// [`Self::serialize`] / [`Self::deserialize`] carry stored points but
    /// intentionally omit the TTL map (see [`Self::serialize`] for the full
    /// contract). For full TTL and snapshot support, create an
    /// [`AgentMemory`](crate::agent::AgentMemory) instead — it owns the shared
    /// `MemoryTtl`, snapshot manager, and all three subsystems.
    ///
    /// # Errors
    ///
    /// Returns an error when collection creation/opening fails or dimensions mismatch.
    pub fn new_from_db(db: Arc<Database>, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension, Arc::new(MemoryTtl::new()))
    }

    pub(crate) fn new(
        db: Arc<Database>,
        dimension: usize,
        ttl: Arc<MemoryTtl>,
    ) -> Result<Self, AgentMemoryError> {
        let (collection_name, dimension, stored_ids) =
            memory_helpers::init_tracked_memory(&db, Self::COLLECTION_NAME, dimension)?;
        memory_helpers::rebuild_ttl_from_payloads(
            &db,
            &collection_name,
            &ttl,
            MemoryKind::Semantic,
        )?;

        let next_edge_id = memory_helpers::seed_edge_counter(&memory_helpers::get_collection(
            &db,
            &collection_name,
        )?);

        Ok(Self {
            collection_name,
            db,
            dimension,
            ttl,
            stored_ids,
            next_edge_id,
        })
    }

    /// Returns the name of the underlying `VelesDB` collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Returns the embedding dimension for this collection.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Ensures a secondary index exists on `field` so filtered recall takes the
    /// indexed bitmap prefilter instead of a linear post-filter scan.
    ///
    /// Idempotent and cheap to call on every filtered query: when the index is
    /// already present this is a single `has_secondary_index` read. The first
    /// call backfills existing payloads and records `field` in the persisted
    /// `indexed_fields` authority (so the index is rebuilt on the next `open`,
    /// not silently lost); subsequent upserts maintain it incrementally.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is missing or persisting the index
    /// authority fails.
    pub fn ensure_index(&self, field: &str) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        if !collection.has_secondary_index(field) {
            collection
                .create_index(field)
                .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
        }
        Ok(())
    }

    /// Stores a semantic memory point.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, collection access fails,
    /// or persistence fails.
    pub fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), AgentMemoryError> {
        self.store_internal(id, content, embedding, None, None)
    }

    /// Stores a semantic memory point with additional metadata fields.
    ///
    /// `content` always wins: if `metadata` contains a `"content"` key, it is
    /// overwritten by the `content` parameter. The reserved system key
    /// `_veles_expires_at` (durable TTL, see [`Self::store_with_ttl`]) is
    /// likewise stripped from `metadata`; a plain `expires_at` key is ordinary
    /// business metadata and is stored verbatim.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::store`].
    pub fn store_with_metadata(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: &Map<String, Value>,
    ) -> Result<(), AgentMemoryError> {
        self.store_internal(id, content, embedding, Some(metadata), None)
    }

    /// Updates payload fields of an existing fact without changing its embedding.
    ///
    /// Only facts that are tracked and not expired are updated. Any key in
    /// `updates` is merged into the existing payload; `content` may be updated
    /// through this method, but the vector is left untouched. The reserved
    /// system key `_veles_expires_at` (durable TTL) is ignored in `updates`
    /// and preserved from the existing payload.
    ///
    /// # Errors
    ///
    /// Returns [`AgentMemoryError::NotFound`] when the id is unknown or expired.
    /// Returns other errors when collection access or persistence fails.
    pub fn update_metadata(
        &self,
        id: u64,
        updates: &Map<String, Value>,
    ) -> Result<(), AgentMemoryError> {
        if !self.stored_ids.read().contains(&id) {
            return Err(AgentMemoryError::NotFound(id.to_string()));
        }
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let point = memory_helpers::ensure_live(
            &collection,
            &self.collection_name,
            &self.ttl,
            MemoryKind::Semantic,
            id,
        )?;
        let payload = merge_payload(point.payload, updates)?;
        memory_helpers::upsert_points(
            &collection,
            vec![Point::new(id, point.vector, Some(payload))],
        )?;
        Ok(())
    }

    /// Shared store path. The durable expiry travels through the dedicated
    /// `expires_at` parameter (written under the reserved
    /// [`memory_helpers::EXPIRES_AT_KEY`]), never through user `metadata`.
    fn store_internal(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: Option<&Map<String, Value>>,
        expires_at: Option<u64>,
    ) -> Result<(), AgentMemoryError> {
        memory_helpers::validate_dimension(self.dimension, embedding.len())?;
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let mut payload = build_payload(content, metadata);
        // Preserve reserved system keys (`_veles_*`: durable TTL, RL confidence,
        // entity tags) from a prior version of this fact, so a content re-store
        // (`remember`) does not silently wipe learned state. Caller metadata is
        // already in `payload` and the explicit `expires_at` below still wins,
        // so a carried-forward value is only used when nothing overwrites it.
        if self.stored_ids.read().contains(&id) {
            if let Ok(existing) = memory_helpers::ensure_live(
                &collection,
                &self.collection_name,
                &self.ttl,
                MemoryKind::Semantic,
                id,
            ) {
                if let (Some(prior), Some(obj)) = (
                    existing.payload.as_ref().and_then(Value::as_object),
                    payload.as_object_mut(),
                ) {
                    for (k, v) in prior {
                        if k.starts_with("_veles_") && !obj.contains_key(k) {
                            obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
        }
        memory_helpers::attach_expiry(&mut payload, expires_at);
        let point = Point::new(id, embedding.to_vec(), Some(payload));
        memory_helpers::upsert_points(&collection, vec![point])?;
        self.stored_ids.write().insert(id);
        Ok(())
    }

    /// Stores a fact under `preferred_id`, or under a freshly allocated id when
    /// `preferred_id` is already taken, and returns the id actually used.
    ///
    /// [`Self::store`] upserts, so reusing an id silently overwrites the
    /// existing fact. Consolidation (which reuses the *episodic* id as the
    /// semantic id) must never clobber an unrelated semantic fact, so it relies
    /// on this collision-avoiding path instead.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::store`].
    pub fn store_unique(
        &self,
        preferred_id: u64,
        content: &str,
        embedding: &[f32],
    ) -> Result<u64, AgentMemoryError> {
        let id = self.allocate_id(preferred_id);
        self.store(id, content, embedding)?;
        Ok(id)
    }

    /// Returns `preferred_id` when free, otherwise the smallest id strictly
    /// greater than every tracked id (so it cannot collide with a live fact).
    fn allocate_id(&self, preferred_id: u64) -> u64 {
        let ids = self.stored_ids.read();
        if !ids.contains(&preferred_id) {
            return preferred_id;
        }
        ids.iter().copied().max().map_or(0, |m| m.saturating_add(1))
    }

    /// Stores a semantic memory point and assigns a TTL.
    ///
    /// A `ttl_seconds` of `0` means "expire immediately": rather than persisting
    /// a live point that then occupies an index slot until the next
    /// `auto_expire`, the point is eagerly removed (and any pre-existing point
    /// for `id` deleted). The embedding is still dimension-validated so callers
    /// get the same error contract as a real store.
    ///
    /// The expiry is persisted as a reserved `_veles_expires_at` (epoch
    /// seconds) payload field, so the TTL survives a process restart: the
    /// in-memory map is rebuilt from payloads when the collection is reopened.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::store`].
    pub fn store_with_ttl(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        ttl_seconds: u64,
    ) -> Result<(), AgentMemoryError> {
        if ttl_seconds == 0 {
            memory_helpers::validate_dimension(self.dimension, embedding.len())?;
            return self.delete(id);
        }
        let expires_at = MemoryTtl::now().saturating_add(ttl_seconds);
        self.store_internal(id, content, embedding, None, Some(expires_at))?;
        self.ttl.set_expiry(MemoryKind::Semantic, id, expires_at);
        Ok(())
    }

    /// Durably sets (or refreshes) the TTL of an existing fact.
    ///
    /// Unlike `AgentMemory::set_semantic_ttl` (in-memory map only, lost on
    /// restart), this persists the expiry to the reserved `_veles_expires_at`
    /// payload field, so it survives a restart. A `ttl_seconds` of 0 expires
    /// the fact immediately.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` when no fact with `id` exists, or `CollectionError`
    /// when persistence fails.
    pub fn set_ttl_durable(&self, id: u64, ttl_seconds: u64) -> Result<(), AgentMemoryError> {
        memory_helpers::set_ttl_durable(
            &self.db,
            &self.collection_name,
            &self.ttl,
            MemoryKind::Semantic,
            id,
            ttl_seconds,
        )
    }

    /// Relates two live facts with a typed, durable graph edge
    /// (`MATCH (a)-[:REL_TYPE]->(b)` becomes executable over this memory).
    ///
    /// Returns the allocated edge id. Edges are WAL-persisted and cascade
    /// away when either endpoint memory is deleted.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` when either endpoint is missing or expired, or
    /// `CollectionError` when the edge write fails.
    pub fn relate(
        &self,
        from_id: u64,
        to_id: u64,
        rel_type: &str,
        properties: Option<&serde_json::Map<String, serde_json::Value>>,
    ) -> Result<u64, AgentMemoryError> {
        memory_helpers::relate_memory_points(
            &memory_helpers::MemorySubsystem {
                db: &self.db,
                collection_name: &self.collection_name,
                ttl: &self.ttl,
                kind: MemoryKind::Semantic,
                next_edge_id: &self.next_edge_id,
            },
            from_id,
            to_id,
            rel_type,
            properties,
        )
    }

    /// Returns the outgoing relations of a fact (edges it points from).
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn relations(
        &self,
        id: u64,
    ) -> Result<Vec<crate::collection::graph::GraphEdge>, AgentMemoryError> {
        memory_helpers::relations_of(
            &self.db,
            &self.collection_name,
            id,
            &self.ttl,
            MemoryKind::Semantic,
        )
    }

    /// Removes a relation edge created by [`Self::relate`].
    ///
    /// Returns `true` when the edge existed and was removed.
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn unrelate(&self, edge_id: u64) -> Result<bool, AgentMemoryError> {
        memory_helpers::unrelate_edge(&self.db, &self.collection_name, edge_id)
    }

    /// Queries semantic memory by vector similarity.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, collection access fails,
    /// or vector search fails.
    pub fn query(
        &self,
        query_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        let results = memory_helpers::search_filtered(
            &self.db,
            &self.collection_name,
            self.dimension,
            query_embedding,
            k,
            &self.ttl,
            MemoryKind::Semantic,
        )?;

        Ok(results
            .into_iter()
            .map(|r| {
                let content = extract_content(&r.point);
                (r.point.id, r.score, content)
            })
            .collect())
    }

    /// Queries semantic memory with a payload filter and optional offset pagination.
    ///
    /// Results are ranked by vector similarity, filtered against `filter` (all
    /// key-value pairs must match), TTL-expired points are excluded, and
    /// `offset` leading results are skipped before taking `k`.
    ///
    /// The internal fetch budget is generous to survive both TTL eviction and
    /// filter miss-rates; when the collection has very few matching entries the
    /// returned slice may be shorter than `k`.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid or collection access fails.
    pub fn query_filtered(
        &self,
        query_embedding: &[f32],
        k: usize,
        filter: &Map<String, Value>,
        offset: usize,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        // over-fetch to absorb TTL evictions + payload filter misses + offset
        let need = k.saturating_add(offset);
        let fetch_k = need
            .saturating_add(self.ttl.expired_count(MemoryKind::Semantic))
            .saturating_mul(2)
            .max(need.saturating_add(8));

        memory_helpers::validate_dimension(self.dimension, query_embedding.len())?;
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let raw = memory_helpers::search_collection(&collection, query_embedding, fetch_k)?;

        Ok(raw
            .into_iter()
            .filter(|r| !self.ttl.is_expired(MemoryKind::Semantic, r.point.id))
            .filter(|r| payload_matches(&r.point, filter))
            .skip(offset)
            .take(k)
            .map(|r| (r.point.id, r.score, extract_content(&r.point)))
            .collect())
    }

    /// Queries semantic memory, dropping points whose payload matches `exclude`.
    ///
    /// A point is excluded when it matches *all* key-value pairs in `exclude`
    /// (the negative counterpart of [`Self::query_filtered`]); an empty `exclude`
    /// drops nothing. Ranked by vector similarity, TTL-expired points removed.
    ///
    /// Unlike a positive filter, an exclude set can be arbitrarily large (e.g.
    /// every internal hub), so a fixed over-fetch could be entirely consumed by
    /// excluded points and return fewer than `k` survivors. To avoid that, the
    /// fetch window **grows geometrically** until `k` survivors are found or the
    /// collection is exhausted — so a real match is never starved out by a dense
    /// band of excluded neighbours.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid or collection access fails.
    pub fn query_excluding(
        &self,
        query_embedding: &[f32],
        k: usize,
        exclude: &Map<String, Value>,
    ) -> Result<Vec<(u64, f32, String)>, AgentMemoryError> {
        memory_helpers::validate_dimension(self.dimension, query_embedding.len())?;
        if exclude.is_empty() || k == 0 {
            return self.query(query_embedding, k);
        }
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let base = k.saturating_add(self.ttl.expired_count(MemoryKind::Semantic));
        let mut fetch_k = base.saturating_mul(2).max(k.saturating_add(8));
        loop {
            let raw = memory_helpers::search_collection(&collection, query_embedding, fetch_k)?;
            let exhausted = raw.len() < fetch_k;
            let kept: Vec<(u64, f32, String)> = raw
                .into_iter()
                .filter(|r| !self.ttl.is_expired(MemoryKind::Semantic, r.point.id))
                .filter(|r| !payload_matches(&r.point, exclude))
                .take(k)
                .map(|r| (r.point.id, r.score, extract_content(&r.point)))
                .collect();
            if kept.len() >= k || exhausted {
                return Ok(kept);
            }
            fetch_k = fetch_k.saturating_mul(2);
        }
    }

    /// Stores multiple semantic memory points in one batch.
    ///
    /// Each tuple is `(id, content, embedding)`. All embeddings are
    /// dimension-validated before any write occurs.
    ///
    /// This is best-effort, not transactional: if `upsert_points` fails partway
    /// the already-persisted points are kept and `stored_ids` is left untouched
    /// (it is only updated after a fully successful upsert), matching the
    /// single-`store` behaviour.
    ///
    /// # Errors
    ///
    /// Returns an error when any embedding dimension is invalid, collection
    /// access fails, or persistence fails.
    pub fn store_batch(&self, facts: &[(u64, &str, &[f32])]) -> Result<(), AgentMemoryError> {
        let mut points = Vec::with_capacity(facts.len());
        for (id, content, embedding) in facts {
            memory_helpers::validate_dimension(self.dimension, embedding.len())?;
            points.push(Point::new(
                *id,
                embedding.to_vec(),
                Some(build_payload(content, None)),
            ));
        }

        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        memory_helpers::upsert_points(&collection, points)?;

        let mut ids = self.stored_ids.write();
        for (id, _, _) in facts {
            ids.insert(*id);
        }
        Ok(())
    }

    /// Retrieves a fact's content and embedding by id.
    ///
    /// Returns `None` when the id is unknown or has expired.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    pub fn get(&self, id: u64) -> Result<Option<(String, Vec<f32>)>, AgentMemoryError> {
        if self.ttl.is_expired(MemoryKind::Semantic, id) {
            return Ok(None);
        }
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let Some(point) = collection.get(&[id]).into_iter().flatten().next() else {
            return Ok(None);
        };
        Ok(Some((extract_content(&point), point.vector.clone())))
    }

    /// Retrieves a fact's raw payload as a metadata map, or `None` when the id
    /// is unknown, expired, or carries no payload. Unlike [`Self::get`], this
    /// skips the embedding entirely, so a caller that only needs to inspect a
    /// fact's tags (e.g. to distinguish internal scaffolding from user data)
    /// doesn't pay for a vector copy.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    pub fn get_metadata(&self, id: u64) -> Result<Option<Map<String, Value>>, AgentMemoryError> {
        if self.ttl.is_expired(MemoryKind::Semantic, id) {
            return Ok(None);
        }
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let Some(point) = collection.get(&[id]).into_iter().flatten().next() else {
            return Ok(None);
        };
        Ok(point.payload.as_ref().and_then(Value::as_object).cloned())
    }

    /// Batched [`Self::get_metadata`]: fetches every id in `ids` with a
    /// single collection lookup, returning results in the same order and
    /// length as `ids` (an unknown or expired id maps to `None`) — avoids
    /// the N individual round trips a per-id loop over `get_metadata` would
    /// cost when a caller needs metadata for a whole batch of hits (e.g.
    /// `velesdb-memory`'s `recall`/`recall_fused`).
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    pub fn get_metadata_batch(
        &self,
        ids: &[u64],
    ) -> Result<Vec<Option<Map<String, Value>>>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let points = collection.get(ids);
        Ok(ids
            .iter()
            .zip(points)
            .map(|(&id, point)| {
                if self.ttl.is_expired(MemoryKind::Semantic, id) {
                    return None;
                }
                point.and_then(|p| p.payload.as_ref().and_then(Value::as_object).cloned())
            })
            .collect())
    }

    /// Lists all live (non-expired) tracked facts as `(id, content)` pairs.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    pub fn list_all(&self) -> Result<Vec<(u64, String)>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();

        Ok(collection
            .get(&all_ids)
            .into_iter()
            .flatten()
            .filter(|p| !self.ttl.is_expired(MemoryKind::Semantic, p.id))
            .map(|p| (p.id, extract_content(&p)))
            .collect())
    }

    /// Returns the number of tracked facts.
    #[must_use]
    pub fn count(&self) -> usize {
        self.stored_ids.read().len()
    }

    /// Returns `true` when no facts are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stored_ids.read().is_empty()
    }

    /// Removes all facts and their tracking entries.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or deletion fails.
    pub fn clear(&self) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
        if !ids.is_empty() {
            memory_helpers::delete_from_collection(&collection, &ids)?;
        }
        for id in &ids {
            self.ttl.remove(MemoryKind::Semantic, *id);
        }
        self.stored_ids.write().clear();
        Ok(())
    }

    /// Deletes a semantic memory point by id.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or deletion fails.
    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        memory_helpers::delete_tracked_point(
            &self.db,
            &self.collection_name,
            id,
            &self.stored_ids,
            &self.ttl,
            MemoryKind::Semantic,
        )
    }

    /// Serializes semantic memory points for snapshot persistence.
    ///
    /// # TTL limitation
    ///
    /// The returned bytes contain only the stored points (id, embedding,
    /// payload — including any durable `_veles_expires_at` field) and intentionally
    /// **omit the TTL map**. TTL is tracked in a single `MemoryTtl` map shared
    /// across the semantic, episodic, and procedural subsystems (see
    /// [`AgentMemory`](crate::agent::AgentMemory)), so it cannot be partitioned
    /// per subsystem here. TTL is persisted and restored globally by
    /// [`AgentMemory::snapshot`](crate::agent::AgentMemory::snapshot) /
    /// `restore_state`. Calling [`Self::deserialize`] in isolation therefore
    /// restores facts but refreshes the in-memory expiry map only at the next
    /// construction (payload `_veles_expires_at` rebuild); use the snapshot manager
    /// for an immediate full round-trip including TTL.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or JSON encoding fails.
    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        memory_helpers::serialize_tracked_points(&self.db, &self.collection_name, &self.stored_ids)
    }

    /// Replaces semantic memory state from snapshot bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON decoding fails, collection access fails,
    /// or persistence operations fail.
    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        memory_helpers::deserialize_tracked_points(
            &self.db,
            &self.collection_name,
            data,
            &self.stored_ids,
        )
    }
}

/// Builds the payload `Value` from `content` and optional extra metadata.
///
/// `content` is always inserted last so it wins over any `"content"` key
/// present in `metadata`. The reserved [`memory_helpers::EXPIRES_AT_KEY`] is
/// stripped: the durable TTL is only ever written by the system store path.
fn build_payload(content: &str, metadata: Option<&Map<String, Value>>) -> Value {
    let mut map = metadata.cloned().unwrap_or_default();
    map.remove(memory_helpers::EXPIRES_AT_KEY);
    map.insert("content".to_string(), Value::String(content.to_string()));
    Value::Object(map)
}

/// Merges `updates` into an existing point payload, returning the new payload.
///
/// A missing payload starts from an empty object. Errors when the existing
/// payload is present but not a JSON object. The reserved
/// [`memory_helpers::EXPIRES_AT_KEY`] is skipped so a metadata update can
/// neither inject nor clobber the durable TTL.
fn merge_payload(
    existing: Option<Value>,
    updates: &Map<String, Value>,
) -> Result<Value, AgentMemoryError> {
    let mut payload = existing.unwrap_or_else(|| Value::Object(Map::new()));
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| AgentMemoryError::IoError("corrupt payload".to_string()))?;
    for (k, v) in updates {
        if k == memory_helpers::EXPIRES_AT_KEY {
            continue;
        }
        obj.insert(k.clone(), v.clone());
    }
    Ok(payload)
}

/// Returns `true` when every key-value pair in `filter` matches the point payload.
///
/// An empty filter matches all points. A point with no payload only matches an
/// empty filter.
fn payload_matches(point: &Point, filter: &Map<String, Value>) -> bool {
    if filter.is_empty() {
        return true;
    }
    let Some(obj) = point.payload.as_ref().and_then(Value::as_object) else {
        return false;
    };
    filter
        .iter()
        .all(|(k, v)| obj.get(k).is_some_and(|pv| pv == v))
}

/// Extracts the `content` string from a point's payload, or `""` when absent.
fn extract_content(point: &Point) -> String {
    point
        .payload
        .as_ref()
        .and_then(|p| p.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
