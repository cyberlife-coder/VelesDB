//! Procedural Memory - Learned patterns storage (US-004)
//!
//! Stores action sequences and learned procedures with confidence scoring.
//! Supports pattern matching by similarity and reinforcement learning.
//! Includes extensible reinforcement strategies for adaptive confidence updates.

use crate::{Database, DistanceMetric, Point};
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::reinforcement::{FixedRate, ReinforcementContext, ReinforcementStrategy};
use super::ttl::MemoryTtl;

#[derive(Debug, Clone)]
pub struct ProcedureMatch {
    pub id: u64,
    pub name: String,
    pub steps: Vec<String>,
    pub confidence: f32,
    pub score: f32,
}

pub struct ProceduralMemory<'a> {
    collection_name: String,
    db: &'a Database,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    reinforcement_strategy: Arc<dyn ReinforcementStrategy>,
    stored_ids: RwLock<HashSet<u64>>,
}

impl<'a> ProceduralMemory<'a> {
    const COLLECTION_NAME: &'static str = "_procedural_memory";

    pub fn new_from_db(db: &'a Database, dimension: usize) -> Result<Self, AgentMemoryError> {
        Self::new(db, dimension, Arc::new(MemoryTtl::new()))
    }

    pub(crate) fn new(
        db: &'a Database,
        dimension: usize,
        ttl: Arc<MemoryTtl>,
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
            reinforcement_strategy: Arc::new(FixedRate::default()),
            stored_ids: RwLock::new(HashSet::new()),
        })
    }

    pub fn with_reinforcement_strategy(mut self, strategy: Arc<dyn ReinforcementStrategy>) -> Self {
        self.reinforcement_strategy = strategy;
        self
    }

    #[must_use]
    pub fn collection_name(&self) -> &str {
        &self.collection_name
    }

    pub fn learn(
        &self,
        procedure_id: u64,
        name: &str,
        steps: &[String],
        embedding: Option<&[f32]>,
        confidence: f32,
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

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let point = Point::new(
            procedure_id,
            vector,
            Some(json!({
                "name": name,
                "steps": steps,
                "confidence": confidence,
                "usage_count": 0,
                "created_at": now,
                "last_used_at": now,
                "success_count": 0,
                "failure_count": 0
            })),
        );

        collection
            .upsert(vec![point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        self.stored_ids.write().insert(procedure_id);
        Ok(())
    }

    pub fn learn_with_ttl(
        &self,
        procedure_id: u64,
        name: &str,
        steps: &[String],
        embedding: Option<&[f32]>,
        confidence: f32,
        ttl_seconds: u64,
    ) -> Result<(), AgentMemoryError> {
        self.learn(procedure_id, name, steps, embedding, confidence)?;
        self.ttl.set_ttl(procedure_id, ttl_seconds);
        Ok(())
    }

    pub fn recall(
        &self,
        query_embedding: &[f32],
        k: usize,
        min_confidence: f32,
    ) -> Result<Vec<ProcedureMatch>, AgentMemoryError> {
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
                let name = payload.get("name")?.as_str()?.to_string();
                let steps: Vec<String> = payload
                    .get("steps")?
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                let confidence = payload.get("confidence")?.as_f64()? as f32;

                if confidence < min_confidence {
                    return None;
                }

                Some(ProcedureMatch {
                    id: r.point.id,
                    name,
                    steps,
                    confidence,
                    score: r.score,
                })
            })
            .collect())
    }

    pub fn reinforce(&self, procedure_id: u64, success: bool) -> Result<(), AgentMemoryError> {
        self.reinforce_with_strategy(procedure_id, success, &*self.reinforcement_strategy)
    }

    pub fn reinforce_with_strategy(
        &self,
        procedure_id: u64,
        success: bool,
        strategy: &dyn ReinforcementStrategy,
    ) -> Result<(), AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let points = collection.get(&[procedure_id]);
        let point = points
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| AgentMemoryError::NotFound(format!("Procedure {procedure_id}")))?;

        let payload = point
            .payload
            .as_ref()
            .ok_or_else(|| AgentMemoryError::CollectionError("Missing payload".to_string()))?;

        let name = payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let steps: Vec<String> = payload
            .get("steps")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let old_confidence = payload
            .get("confidence")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.5) as f32;
        let usage_count = payload
            .get("usage_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let created_at = payload
            .get("created_at")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        let success_count = payload
            .get("success_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let failure_count = payload
            .get("failure_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let total_uses = success_count + failure_count;
        let success_rate = if total_uses > 0 {
            success_count as f32 / total_uses as f32
        } else {
            0.5
        };

        let context = ReinforcementContext {
            usage_count,
            created_at: created_at as u64,
            last_used: now as u64,
            current_time: now as u64,
            recent_success_rate: Some(success_rate),
            custom: std::collections::HashMap::new(),
        };

        let new_confidence = strategy.update_confidence(old_confidence, success, &context);

        let (new_success, new_failure) = if success {
            (success_count + 1, failure_count)
        } else {
            (success_count, failure_count + 1)
        };

        let updated_point = Point::new(
            procedure_id,
            point.vector.clone(),
            Some(json!({
                "name": name,
                "steps": steps,
                "confidence": new_confidence,
                "usage_count": usage_count + 1,
                "created_at": created_at,
                "last_used_at": now,
                "success_count": new_success,
                "failure_count": new_failure
            })),
        );

        collection
            .upsert(vec![updated_point])
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(())
    }

    pub fn list_all(&self) -> Result<Vec<ProcedureMatch>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
        let points = collection.get(&all_ids);

        Ok(points
            .into_iter()
            .flatten()
            .filter(|p| !self.ttl.is_expired(p.id))
            .filter_map(|p| {
                let payload = p.payload.as_ref()?;
                let name = payload.get("name")?.as_str()?.to_string();
                let steps: Vec<String> = payload
                    .get("steps")?
                    .as_array()?
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                let confidence = payload.get("confidence")?.as_f64()? as f32;

                Some(ProcedureMatch {
                    id: p.id,
                    name,
                    steps,
                    confidence,
                    score: 0.0,
                })
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

        self.stored_ids.write().remove(&id);
        self.ttl.remove(id);
        Ok(())
    }

    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        let collection = self
            .db
            .get_collection(&self.collection_name)
            .ok_or_else(|| AgentMemoryError::CollectionError("Collection not found".to_string()))?;

        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
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

        {
            let mut ids = self.stored_ids.write();
            ids.clear();
            for point in &points {
                ids.insert(point.id);
            }
        }

        collection
            .upsert(points)
            .map_err(|e| AgentMemoryError::CollectionError(e.to_string()))?;

        Ok(())
    }
}
