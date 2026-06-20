//! Episodic Memory - Event timeline storage (US-003)
//!
//! Records events with timestamps and contextual information.
//! Supports temporal queries and similarity-based retrieval.
//! Uses a B-tree temporal index for efficient O(log N) time-based queries.

use crate::{Database, Point};
use serde_json::json;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::memory_helpers;
use super::temporal_index::TemporalIndex;
use super::ttl::{MemoryKind, MemoryTtl};

/// Episodic memory for storing event timelines with temporal context.
///
/// Records events with timestamps, descriptions, and embeddings.
/// Supports similarity-based retrieval and time-range queries.
pub struct EpisodicMemory {
    collection_name: String,
    db: Arc<Database>,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    temporal_index: Arc<TemporalIndex>,
    /// Edge-id allocator for [`Self::relate`] (seeded past existing edges).
    next_edge_id: std::sync::atomic::AtomicU64,
}

impl EpisodicMemory {
    const COLLECTION_NAME: &'static str = "_episodic_memory";

    /// Returns the embedding dimension for this collection.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Creates or opens the episodic memory collection.
    ///
    /// # Errors
    ///
    /// Returns an error when collection creation/opening fails or dimensions mismatch.
    pub fn new_from_db(db: Arc<Database>, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(
            db,
            dimension,
            Arc::new(MemoryTtl::new()),
            Arc::new(TemporalIndex::new()),
        )
    }
    pub(crate) fn new(
        db: Arc<Database>,
        dimension: usize,
        ttl: Arc<MemoryTtl>,
        temporal_index: Arc<TemporalIndex>,
    ) -> Result<Self, AgentMemoryError> {
        let collection_name = Self::COLLECTION_NAME.to_string();
        let actual_dimension =
            memory_helpers::open_or_create_collection(&db, &collection_name, dimension)?;

        if temporal_index.is_empty() {
            if let Some(collection) = db.get_vector_collection(&collection_name) {
                Self::rebuild_temporal_index(&collection.inner, &temporal_index);
            }
        }
        memory_helpers::rebuild_ttl_from_payloads(
            &db,
            &collection_name,
            &ttl,
            MemoryKind::Episodic,
        )?;

        let next_edge_id = memory_helpers::seed_edge_counter(&memory_helpers::get_collection(
            &db,
            &collection_name,
        )?);

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
            ttl,
            temporal_index,
            next_edge_id,
        })
    }
    fn rebuild_temporal_index(
        collection: &crate::collection::Collection,
        temporal_index: &TemporalIndex,
    ) {
        let all_ids = collection.all_ids();
        let points = collection.get(&all_ids);
        for point in points.into_iter().flatten() {
            if let Some(payload) = &point.payload {
                if let Some(ts) = payload.get("timestamp").and_then(serde_json::Value::as_i64) {
                    temporal_index.insert(point.id, ts);
                }
            }
        }
    }

    /// Returns the name of the underlying `VelesDB` collection.
    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    /// Stores an event in episodic memory.
    ///
    /// # Errors
    ///
    /// Returns an error when the embedding dimension is invalid, when the collection
    /// is unavailable, or when storage upsert fails.
    pub fn record(
        &self,
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<&[f32]>,
    ) -> Result<(), AgentMemoryError> {
        self.record_internal(event_id, description, timestamp, embedding, None)
    }

    /// Shared store path: persists the event, optionally with a durable
    /// `_veles_expires_at` payload field (epoch seconds) for TTL'd records.
    fn record_internal(
        &self,
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<&[f32]>,
        expires_at: Option<u64>,
    ) -> Result<(), AgentMemoryError> {
        let vector = memory_helpers::resolve_embedding(self.dimension, embedding)?;
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        let mut payload = json!({
            "description": description,
            "timestamp": timestamp
        });
        memory_helpers::attach_expiry(&mut payload, expires_at);
        let point = Point::new(event_id, vector, Some(payload));

        memory_helpers::upsert_points(&collection, vec![point])?;
        self.temporal_index.insert(event_id, timestamp);

        Ok(())
    }

    /// Stores an event and assigns a TTL for automatic expiration.
    ///
    /// A `ttl_seconds` of `0` means "expire immediately": rather than persisting
    /// a live point that lingers until the next `auto_expire`, the event is
    /// eagerly removed (and any pre-existing point for `event_id` deleted),
    /// harmonising the behaviour with `SemanticMemory::store_with_ttl`. The
    /// embedding is still dimension-validated so callers get the same error
    /// contract as a real record.
    ///
    /// The expiry is persisted as a reserved `_veles_expires_at` (epoch
    /// seconds) payload field, so the TTL survives a process restart: the
    /// in-memory map is rebuilt from payloads when the collection is reopened.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::record`].
    pub fn record_with_ttl(
        &self,
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<&[f32]>,
        ttl_seconds: u64,
    ) -> Result<(), AgentMemoryError> {
        if ttl_seconds == 0 {
            if let Some(emb) = embedding {
                memory_helpers::validate_dimension(self.dimension, emb.len())?;
            }
            return self.delete(event_id);
        }
        let expires_at = MemoryTtl::now().saturating_add(ttl_seconds);
        self.record_internal(
            event_id,
            description,
            timestamp,
            embedding,
            Some(expires_at),
        )?;
        self.ttl
            .set_expiry(MemoryKind::Episodic, event_id, expires_at);
        Ok(())
    }

    /// Durably sets (or refreshes) the TTL of an existing event.
    ///
    /// Unlike `AgentMemory::set_episodic_ttl` (in-memory map only, lost on
    /// restart), this persists the expiry to the reserved `_veles_expires_at`
    /// payload field, so it survives a restart. A `ttl_seconds` of 0 expires
    /// the event immediately.
    ///
    /// # Errors
    ///
    /// Returns `NotFound` when no event with `event_id` exists, or
    /// `CollectionError` when persistence fails.
    pub fn set_ttl_durable(&self, event_id: u64, ttl_seconds: u64) -> Result<(), AgentMemoryError> {
        memory_helpers::set_ttl_durable(
            &self.db,
            &self.collection_name,
            &self.ttl,
            MemoryKind::Episodic,
            event_id,
            ttl_seconds,
        )
    }

    /// Relates two live events with a typed, durable graph edge (e.g.
    /// `CAUSED`, `FOLLOWED`); see `SemanticMemory::relate` for semantics.
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
                kind: MemoryKind::Episodic,
                next_edge_id: &self.next_edge_id,
            },
            from_id,
            to_id,
            rel_type,
            properties,
        )
    }

    /// Returns the outgoing relations of an event.
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
            MemoryKind::Episodic,
        )
    }

    /// Removes a relation edge created by [`Self::relate`].
    ///
    /// # Errors
    ///
    /// Returns `CollectionError` when the collection cannot be resolved.
    pub fn unrelate(&self, edge_id: u64) -> Result<bool, AgentMemoryError> {
        memory_helpers::unrelate_edge(&self.db, &self.collection_name, edge_id)
    }

    /// Returns recent events, optionally filtered by a lower timestamp bound.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable.
    pub fn recent(
        &self,
        limit: usize,
        since_timestamp: Option<i64>,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        Ok(self.fetch_temporal_events(
            limit,
            |fetch_limit| {
                let entries = self.temporal_index.recent(fetch_limit, since_timestamp);
                entries.iter().map(|e| e.id).collect()
            },
            &collection,
        ))
    }

    /// Returns events older than `timestamp`.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable.
    pub fn older_than(
        &self,
        timestamp: i64,
        limit: usize,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        Ok(self.fetch_temporal_events(
            limit,
            |fetch_limit| {
                let entries = self.temporal_index.older_than(timestamp, fetch_limit);
                entries.iter().map(|e| e.id).collect()
            },
            &collection,
        ))
    }

    /// Retrieves the `k` most similar episodic events to a query embedding.
    ///
    /// # Errors
    ///
    /// Returns an error when the embedding dimension is invalid, when the collection
    /// is unavailable, or when vector search fails.
    pub fn recall_similar(
        &self,
        query_embedding: &[f32],
        k: usize,
    ) -> Result<Vec<(u64, String, i64, f32)>, AgentMemoryError> {
        let results = memory_helpers::search_filtered(
            &self.db,
            &self.collection_name,
            self.dimension,
            query_embedding,
            k,
            &self.ttl,
            MemoryKind::Episodic,
        )?;

        Ok(results
            .into_iter()
            .filter_map(|r| {
                let (desc, ts) = extract_event_fields(&r.point)?;
                Some((r.point.id, desc, ts, r.score))
            })
            .collect())
    }

    /// Retrieves an event with its embedding payload.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable.
    pub fn get_with_embedding(
        &self,
        id: u64,
    ) -> Result<Option<(String, i64, Vec<f32>)>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        let points = collection.get(&[id]);
        let Some(point) = points.into_iter().flatten().next() else {
            return Ok(None);
        };

        if self.ttl.is_expired(MemoryKind::Episodic, point.id) {
            return Ok(None);
        }

        let Some(payload) = point.payload.as_ref() else {
            return Ok(None);
        };

        let desc = payload
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let ts = payload
            .get("timestamp")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        Ok(Some((desc, ts, point.vector.clone())))
    }

    /// Deletes an episodic event by id.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable or delete fails.
    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        memory_helpers::delete_from_collection(&collection, &[id])?;

        self.temporal_index.remove(id);
        self.ttl.remove(MemoryKind::Episodic, id);
        Ok(())
    }

    /// Serializes episodic points in temporal-order id set.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable or JSON encoding fails.
    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        let all_ids = self.temporal_index.all_ids();
        memory_helpers::serialize_points(&collection, &all_ids)
    }

    /// Replaces episodic storage with previously serialized points.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON decoding fails, collection access fails, or
    /// persistence operations fail.
    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        if let Some(points) = memory_helpers::deserialize_into_collection(data, &collection)? {
            self.rebuild_temporal_from_points(&points);
        }
        Ok(())
    }

    /// Fetches temporal events with progressive widening, filtering expired entries.
    fn fetch_temporal_events(
        &self,
        limit: usize,
        id_fetcher: impl Fn(usize) -> Vec<u64>,
        collection: &crate::collection::Collection,
    ) -> Vec<(u64, String, i64)> {
        // Clamp the pre-allocation: the number of events can never exceed the
        // total indexed entries, so a huge caller-supplied `limit` must not
        // pre-allocate beyond available data.
        let indexed = self.temporal_index.len();
        let mut events = Vec::with_capacity(limit.min(indexed));
        let mut fetch_limit = limit.saturating_mul(2);
        // Saturating to keep an attacker-supplied `limit` near `usize::MAX` from
        // overflowing the loop ceiling (panic under `panic=abort`, silent wrap in
        // release). The `id_count < fetch_limit` break still terminates the loop.
        let max_fetch = indexed.max(limit).saturating_mul(2);

        while events.len() < limit && fetch_limit <= max_fetch {
            let ids = id_fetcher(fetch_limit);
            if ids.is_empty() {
                break;
            }
            let id_count = ids.len();

            events = Self::filter_live_events(&self.ttl, collection, &ids, limit);

            if events.len() >= limit || id_count < fetch_limit {
                break;
            }
            fetch_limit = fetch_limit.saturating_mul(2);
        }

        events
    }

    /// Fetches points by IDs, filters expired ones, and extracts event fields.
    fn filter_live_events(
        ttl: &MemoryTtl,
        collection: &crate::collection::Collection,
        ids: &[u64],
        limit: usize,
    ) -> Vec<(u64, String, i64)> {
        collection
            .get(ids)
            .into_iter()
            .flatten()
            .filter(|p| !ttl.is_expired(MemoryKind::Episodic, p.id))
            .filter_map(|p| {
                let (desc, ts) = extract_event_fields(&p)?;
                Some((p.id, desc, ts))
            })
            .take(limit)
            .collect()
    }

    /// Clears and rebuilds the temporal index from a set of points.
    fn rebuild_temporal_from_points(&self, points: &[Point]) {
        self.temporal_index.clear();
        for point in points {
            if let Some(payload) = &point.payload {
                if let Some(ts) = payload.get("timestamp").and_then(serde_json::Value::as_i64) {
                    self.temporal_index.insert(point.id, ts);
                }
            }
        }
    }
}

/// Extracts `(description, timestamp)` from a point's payload.
fn extract_event_fields(point: &Point) -> Option<(String, i64)> {
    let payload = point.payload.as_ref()?;
    let desc = payload.get("description")?.as_str()?.to_string();
    let ts = payload.get("timestamp")?.as_i64()?;
    Some((desc, ts))
}
