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
use super::ttl::MemoryTtl;

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

    #[allow(deprecated)]
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
            if let Some(collection) = db.get_collection(&collection_name) {
                Self::rebuild_temporal_index(&collection, &temporal_index);
            }
        }

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
            ttl,
            temporal_index,
        })
    }

    #[allow(deprecated)] // Uses legacy Collection internally.
    fn rebuild_temporal_index(collection: &crate::Collection, temporal_index: &TemporalIndex) {
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
    #[allow(deprecated)]
    pub fn record(
        &self,
        event_id: u64,
        description: &str,
        timestamp: i64,
        embedding: Option<&[f32]>,
    ) -> Result<(), AgentMemoryError> {
        let vector = memory_helpers::resolve_embedding(self.dimension, embedding)?;
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        let point = Point::new(
            event_id,
            vector,
            Some(json!({
                "description": description,
                "timestamp": timestamp
            })),
        );

        memory_helpers::upsert_points(&collection, vec![point])?;
        self.temporal_index.insert(event_id, timestamp);

        Ok(())
    }

    /// Stores an event and assigns a TTL for automatic expiration.
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
        self.record(event_id, description, timestamp, embedding)?;
        self.ttl.set_ttl(event_id, ttl_seconds);
        Ok(())
    }

    /// Returns recent events, optionally filtered by a lower timestamp bound.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable.
    #[allow(deprecated)]
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
    #[allow(deprecated)]
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
    #[allow(deprecated)]
    pub fn get_with_embedding(
        &self,
        id: u64,
    ) -> Result<Option<(String, i64, Vec<f32>)>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        let points = collection.get(&[id]);
        let Some(point) = points.into_iter().flatten().next() else {
            return Ok(None);
        };

        if self.ttl.is_expired(point.id) {
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
    #[allow(deprecated)]
    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        memory_helpers::delete_from_collection(&collection, &[id])?;

        self.temporal_index.remove(id);
        self.ttl.remove(id);
        Ok(())
    }

    /// Serializes episodic points in temporal-order id set.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable or JSON encoding fails.
    #[allow(deprecated)]
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
    #[allow(deprecated)]
    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;
        if let Some(points) = memory_helpers::deserialize_into_collection(data, &collection)? {
            self.rebuild_temporal_from_points(&points);
        }
        Ok(())
    }

    /// Fetches temporal events with progressive widening, filtering expired entries.
    #[allow(deprecated)]
    fn fetch_temporal_events(
        &self,
        limit: usize,
        id_fetcher: impl Fn(usize) -> Vec<u64>,
        collection: &crate::Collection,
    ) -> Vec<(u64, String, i64)> {
        let mut events = Vec::with_capacity(limit);
        let mut fetch_limit = limit * 2;
        let max_fetch = self.temporal_index.len().max(limit);

        while events.len() < limit && fetch_limit <= max_fetch * 2 {
            let ids = id_fetcher(fetch_limit);
            if ids.is_empty() {
                break;
            }
            let id_count = ids.len();

            events = Self::filter_live_events(&self.ttl, collection, &ids, limit);

            if events.len() >= limit || id_count < fetch_limit {
                break;
            }
            fetch_limit *= 2;
        }

        events
    }

    /// Fetches points by IDs, filters expired ones, and extracts event fields.
    #[allow(deprecated)]
    fn filter_live_events(
        ttl: &MemoryTtl,
        collection: &crate::Collection,
        ids: &[u64],
        limit: usize,
    ) -> Vec<(u64, String, i64)> {
        collection
            .get(ids)
            .into_iter()
            .flatten()
            .filter(|p| !ttl.is_expired(p.id))
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
