//! Episodic Memory - Event timeline storage (US-003)
#![allow(missing_docs)] // Documentation will be added in follow-up PR
//!
//! Records events with timestamps and contextual information.
//! Supports temporal queries and similarity-based retrieval.
//! Uses a B-tree temporal index for efficient O(log N) time-based queries.

use crate::{Database, DistanceMetric, Point};
use serde_json::json;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::temporal_index::TemporalIndex;
use super::ttl::MemoryTtl;

/// Episodic memory for storing event timelines with temporal context.
///
/// Records events with timestamps, descriptions, and embeddings.
/// Supports similarity-based retrieval and time-range queries.
pub struct EpisodicMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    temporal_index: Arc<TemporalIndex>,
}

impl<'a> EpisodicMemory<'a> {
    const COLLECTION_NAME: &'static str = "_episodic_memory";

    /// Creates or opens the episodic memory collection.
    ///
    /// # Errors
    ///
    /// Returns an error when collection creation/opening fails or dimensions mismatch.
    pub fn new_from_db(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(
            db,
            dimension,
            Arc::new(MemoryTtl::new()),
            Arc::new(TemporalIndex::new()),
        )
    }

    pub(crate) fn new(
        db: &'a Database,
        dimension: usize,
        ttl: Arc<MemoryTtl>,
        temporal_index: Arc<TemporalIndex>,
    ) -> Result<Self, AgentMemoryError> {
        let collection_name = Self::COLLECTION_NAME.to_string();

        let actual_dimension = if let Some(collection) = db.get_collection(&collection_name) {
            let existing_dim = collection.config().dimension;
            if existing_dim != dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: existing_dim,
                    actual: dimension,
                });
            }

            if temporal_index.is_empty() {
                Self::rebuild_temporal_index(&collection, &temporal_index);
            }

            existing_dim
        } else {
            db.create_collection(&collection_name, dimension, DistanceMetric::Cosine)?;
            dimension
        };

        Ok(Self {
            collection_name,
            db,
            dimension: actual_dimension,
            ttl,
            temporal_index,
        })
    }

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
        if let Some(emb) = embedding {
            if emb.len() != self.dimension {
                return Err(AgentMemoryError::DimensionMismatch {
                    expected: self.dimension,
                    actual: emb.len(),
                });
            }
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let vector = embedding.map_or_else(|| vec![0.0; self.dimension], <[f32]>::to_vec);

        let point = Point::new(
            event_id,
            vector,
            Some(json!({
                "description": description,
                "timestamp": timestamp
            })),
        );

        collection
            .upsert(vec![point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

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
    pub fn recent(
        &self,
        limit: usize,
        since_timestamp: Option<i64>,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let mut events = Vec::with_capacity(limit);
        let mut fetch_limit = limit * 2;
        let max_fetch = self.temporal_index.len().max(limit);

        while events.len() < limit && fetch_limit <= max_fetch * 2 {
            let recent_entries = self.temporal_index.recent(fetch_limit, since_timestamp);
            let recent_ids: Vec<u64> = recent_entries.iter().map(|e| e.id).collect();

            if recent_ids.is_empty() {
                break;
            }

            let points = collection.get(&recent_ids);

            events = points
                .into_iter()
                .flatten()
                .filter(|p| !self.ttl.is_expired(p.id))
                .filter_map(|p| {
                    let payload = p.payload.as_ref()?;
                    let desc = payload.get("description")?.as_str()?.to_string();
                    let ts = payload.get("timestamp")?.as_i64()?;
                    Some((p.id, desc, ts))
                })
                .take(limit)
                .collect();

            if events.len() >= limit || recent_ids.len() < fetch_limit {
                break;
            }

            fetch_limit *= 2;
        }

        Ok(events)
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
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let mut events = Vec::with_capacity(limit);
        let mut fetch_limit = limit * 2;
        let max_fetch = self.temporal_index.len().max(limit);

        while events.len() < limit && fetch_limit <= max_fetch * 2 {
            let old_entries = self.temporal_index.older_than(timestamp, fetch_limit);
            let old_ids: Vec<u64> = old_entries.iter().map(|e| e.id).collect();

            if old_ids.is_empty() {
                break;
            }

            let points = collection.get(&old_ids);

            events = points
                .into_iter()
                .flatten()
                .filter(|p| !self.ttl.is_expired(p.id))
                .filter_map(|p| {
                    let payload = p.payload.as_ref()?;
                    let desc = payload.get("description")?.as_str()?.to_string();
                    let ts = payload.get("timestamp")?.as_i64()?;
                    Some((p.id, desc, ts))
                })
                .take(limit)
                .collect();

            if events.len() >= limit || old_ids.len() < fetch_limit {
                break;
            }

            fetch_limit *= 2;
        }

        Ok(events)
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
        if query_embedding.len() != self.dimension {
            return Err(AgentMemoryError::DimensionMismatch {
                expected: self.dimension,
                actual: query_embedding.len(),
            });
        }

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let results = collection
            .search(query_embedding, k)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(results
            .into_iter()
            .filter(|r| !self.ttl.is_expired(r.point.id))
            .filter_map(|r| {
                let payload = r.point.payload.as_ref()?;
                let desc = payload.get("description")?.as_str()?.to_string();
                let ts = payload.get("timestamp")?.as_i64()?;
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
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

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
    pub fn delete(&self, id: u64) -> Result<(), AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        collection
            .delete(&[id])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        self.temporal_index.remove(id);
        self.ttl.remove(id);
        Ok(())
    }

    /// Serializes episodic points in temporal-order id set.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection is unavailable or JSON encoding fails.
    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let all_ids = self.temporal_index.all_ids();
        let points: Vec<_> = collection.get(&all_ids).into_iter().flatten().collect();

        let serialized =
            serde_json::to_vec(&points).map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

        Ok(serialized)
    }

    /// Replaces episodic storage with previously serialized points.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON decoding fails, collection access fails, or
    /// persistence operations fail.
    pub fn deserialize(&self, data: &[u8]) -> Result<(), AgentMemoryError> {
        if data.is_empty() {
            return Ok(());
        }

        let points: Vec<Point> =
            serde_json::from_slice(data).map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let existing_ids = collection.all_ids();
        if !existing_ids.is_empty() {
            collection
                .delete(&existing_ids)
                .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;
        }

        self.temporal_index.clear();
        for point in &points {
            if let Some(payload) = &point.payload {
                if let Some(ts) = payload.get("timestamp").and_then(serde_json::Value::as_i64) {
                    self.temporal_index.insert(point.id, ts);
                }
            }
        }

        collection
            .upsert(points)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(())
    }
}
