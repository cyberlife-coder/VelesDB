//! Episodic Memory - Event timeline storage (US-003)
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

pub struct EpisodicMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    temporal_index: Arc<TemporalIndex>,
}

impl<'a> EpisodicMemory<'a> {
    const COLLECTION_NAME: &'static str = "_episodic_memory";

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

    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

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

    pub fn recent(
        &self,
        limit: usize,
        since_timestamp: Option<i64>,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let recent_entries = self.temporal_index.recent(limit, since_timestamp);
        let recent_ids: Vec<u64> = recent_entries.iter().map(|e| e.id).collect();

        let points = collection.get(&recent_ids);

        let events: Vec<(u64, String, i64)> = points
            .into_iter()
            .flatten()
            .filter(|p| !self.ttl.is_expired(p.id))
            .filter_map(|p| {
                let payload = p.payload.as_ref()?;
                let desc = payload.get("description")?.as_str()?.to_string();
                let ts = payload.get("timestamp")?.as_i64()?;
                Some((p.id, desc, ts))
            })
            .collect();

        Ok(events)
    }

    pub fn older_than(
        &self,
        timestamp: i64,
        limit: usize,
    ) -> Result<Vec<(u64, String, i64)>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let old_entries = self.temporal_index.older_than(timestamp, limit);
        let old_ids: Vec<u64> = old_entries.iter().map(|e| e.id).collect();

        let points = collection.get(&old_ids);

        let events: Vec<(u64, String, i64)> = points
            .into_iter()
            .flatten()
            .filter(|p| !self.ttl.is_expired(p.id))
            .filter_map(|p| {
                let payload = p.payload.as_ref()?;
                let desc = payload.get("description")?.as_str()?.to_string();
                let ts = payload.get("timestamp")?.as_i64()?;
                Some((p.id, desc, ts))
            })
            .collect();

        Ok(events)
    }

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

    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let all_ids: Vec<u64> = (0..100000).collect();
        let points: Vec<_> = collection.get(&all_ids).into_iter().flatten().collect();

        let serialized =
            serde_json::to_vec(&points).map_err(|e| AgentMemoryError::IoError(e.to_string()))?;

        Ok(serialized)
    }

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

        for point in &points {
            if let Some(payload) = &point.payload {
                if let Some(ts) = payload.get("timestamp").and_then(|v| v.as_i64()) {
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
