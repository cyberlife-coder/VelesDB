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
}

impl SemanticMemory {
    const COLLECTION_NAME: &'static str = "_semantic_memory";

    /// Creates or opens semantic memory with an **independent** in-memory TTL.
    ///
    /// # Standalone limitation
    ///
    /// The [`MemoryTtl`] allocated here is not shared with any snapshot
    /// mechanism. TTLs assigned at store time ([`Self::store_with_ttl`]) are
    /// durable: the expiry is persisted as an `expires_at` payload field and
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

        Ok(Self {
            collection_name,
            db,
            dimension,
            ttl,
            stored_ids,
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

    /// Stores a semantic memory point.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, collection access fails,
    /// or persistence fails.
    pub fn store(&self, id: u64, content: &str, embedding: &[f32]) -> Result<(), AgentMemoryError> {
        self.store_internal(id, content, embedding, None)
    }

    /// Stores a semantic memory point with additional metadata fields.
    ///
    /// `content` always wins: if `metadata` contains a `"content"` key, it is
    /// overwritten by the `content` parameter.
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
        self.store_internal(id, content, embedding, Some(metadata))
    }

    /// Updates payload fields of an existing fact without changing its embedding.
    ///
    /// Only facts that are tracked and not expired are updated. Any key in
    /// `updates` is merged into the existing payload; `content` may be updated
    /// through this method, but the vector is left untouched.
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
        if self.ttl.is_expired(MemoryKind::Semantic, id) || !self.stored_ids.read().contains(&id) {
            return Err(AgentMemoryError::NotFound(id.to_string()));
        }
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let Some(point) = collection.get(&[id]).into_iter().flatten().next() else {
            return Err(AgentMemoryError::NotFound(id.to_string()));
        };
        let payload = merge_payload(point.payload, updates)?;
        memory_helpers::upsert_points(
            &collection,
            vec![Point::new(id, point.vector, Some(payload))],
        )?;
        Ok(())
    }

    fn store_internal(
        &self,
        id: u64,
        content: &str,
        embedding: &[f32],
        metadata: Option<&Map<String, Value>>,
    ) -> Result<(), AgentMemoryError> {
        memory_helpers::validate_dimension(self.dimension, embedding.len())?;
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let point = Point::new(
            id,
            embedding.to_vec(),
            Some(build_payload(content, metadata)),
        );
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
    /// The expiry is persisted as an `expires_at` (epoch seconds) payload field,
    /// so the TTL survives a process restart: the in-memory map is rebuilt from
    /// payloads when the collection is reopened.
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
        let mut metadata = Map::new();
        metadata.insert(
            memory_helpers::EXPIRES_AT_KEY.to_string(),
            Value::from(expires_at),
        );
        self.store_internal(id, content, embedding, Some(&metadata))?;
        self.ttl.set_expiry(MemoryKind::Semantic, id, expires_at);
        Ok(())
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
    /// payload — including any durable `expires_at` field) and intentionally
    /// **omit the TTL map**. TTL is tracked in a single `MemoryTtl` map shared
    /// across the semantic, episodic, and procedural subsystems (see
    /// [`AgentMemory`](crate::agent::AgentMemory)), so it cannot be partitioned
    /// per subsystem here. TTL is persisted and restored globally by
    /// [`AgentMemory::snapshot`](crate::agent::AgentMemory::snapshot) /
    /// `restore_state`. Calling [`Self::deserialize`] in isolation therefore
    /// restores facts but refreshes the in-memory expiry map only at the next
    /// construction (payload `expires_at` rebuild); use the snapshot manager
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
/// present in `metadata`.
fn build_payload(content: &str, metadata: Option<&Map<String, Value>>) -> Value {
    let mut map = metadata.cloned().unwrap_or_default();
    map.insert("content".to_string(), Value::String(content.to_string()));
    Value::Object(map)
}

/// Merges `updates` into an existing point payload, returning the new payload.
///
/// A missing payload starts from an empty object. Errors when the existing
/// payload is present but not a JSON object.
fn merge_payload(
    existing: Option<Value>,
    updates: &Map<String, Value>,
) -> Result<Value, AgentMemoryError> {
    let mut payload = existing.unwrap_or_else(|| Value::Object(Map::new()));
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| AgentMemoryError::IoError("corrupt payload".to_string()))?;
    for (k, v) in updates {
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
