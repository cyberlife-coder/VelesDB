//! Procedural Memory - Learned patterns storage (US-004)
//!
//! Stores action sequences and learned procedures with confidence scoring.
//! Supports pattern matching by similarity and reinforcement learning.
//! Includes extensible reinforcement strategies for adaptive confidence updates.

// SAFETY: Numeric casts in procedural memory are intentional:
// - u64->i64 casts for timestamps (SystemTime::elapsed returns u64, DB uses i64)
// - i64->u64 casts for display purposes (timestamps are always positive)
// - f64->f32 casts for confidence scores (f32 precision sufficient, values clamped to 0.0-1.0)
// - All timestamp values are bounded by reasonable time ranges
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use crate::{Database, Point};
use parking_lot::RwLock;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;

use super::error::AgentMemoryError;
use super::memory_helpers;
use super::reinforcement::{FixedRate, ReinforcementContext, ReinforcementStrategy};
use super::ttl::MemoryTtl;

struct ProcedureState {
    name: String,
    steps: Vec<String>,
    confidence: f32,
    usage_count: u64,
    created_at: i64,
    success_count: u64,
    failure_count: u64,
}

impl ProcedureState {
    fn build_reinforcement_context(&self, now: i64) -> ReinforcementContext {
        let total_uses = self.success_count + self.failure_count;
        let success_rate = if total_uses > 0 {
            self.success_count as f32 / total_uses as f32
        } else {
            0.5
        };
        ReinforcementContext {
            usage_count: self.usage_count,
            created_at: self.created_at as u64,
            last_used: now as u64,
            current_time: now as u64,
            recent_success_rate: Some(success_rate),
            custom: std::collections::HashMap::new(),
        }
    }
}

/// A procedure match result returned by [`ProceduralMemory::recall`].
#[derive(Debug, Clone)]
pub struct ProcedureMatch {
    /// Unique identifier for the procedure.
    pub id: u64,
    /// Human-readable name for the procedure.
    pub name: String,
    /// Ordered sequence of steps that constitute the procedure.
    pub steps: Vec<String>,
    /// Confidence score of this procedure (0.0 - 1.0).
    pub confidence: f32,
    /// Similarity score from the vector search.
    pub score: f32,
}

/// Procedural memory for storing learned action sequences with confidence scoring.
///
/// Stores procedures as embedding vectors with associated metadata.
/// Supports confidence-based recall, reinforcement learning, and TTL expiration.
pub struct ProceduralMemory {
    collection_name: String,
    db: Arc<Database>,
    dimension: usize,
    ttl: Arc<MemoryTtl>,
    reinforcement_strategy: Arc<dyn ReinforcementStrategy>,
    stored_ids: RwLock<HashSet<u64>>,
}

impl ProceduralMemory {
    const COLLECTION_NAME: &'static str = "_procedural_memory";

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

    /// Creates or opens procedural memory.
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

        Ok(Self {
            collection_name,
            db,
            dimension,
            ttl,
            reinforcement_strategy: Arc::new(FixedRate::default()),
            stored_ids,
        })
    }

    /// Overrides the default reinforcement strategy with a custom implementation.
    #[must_use]
    pub fn with_reinforcement_strategy(mut self, strategy: Arc<dyn ReinforcementStrategy>) -> Self {
        self.reinforcement_strategy = strategy;
        self
    }

    /// Learns a procedure and stores it in memory.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, the collection is
    /// unavailable, or persistence fails.
    #[allow(deprecated)]
    pub fn learn(
        &self,
        procedure_id: u64,
        name: &str,
        steps: &[String],
        embedding: Option<&[f32]>,
        confidence: f32,
    ) -> Result<(), AgentMemoryError> {
        let vector = memory_helpers::resolve_embedding(self.dimension, embedding)?;
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

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

        memory_helpers::upsert_points(&collection, vec![point])?;
        self.stored_ids.write().insert(procedure_id);
        Ok(())
    }

    /// Learns a procedure and assigns a TTL for auto-expiration.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::learn`].
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

    /// Recalls matching procedures by vector similarity.
    ///
    /// # Errors
    ///
    /// Returns an error when embedding dimension is invalid, collection access fails,
    /// or vector search fails.
    pub fn recall(
        &self,
        query_embedding: &[f32],
        k: usize,
        min_confidence: f32,
    ) -> Result<Vec<ProcedureMatch>, AgentMemoryError> {
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
                let pm = extract_procedure_match(&r.point, r.score)?;
                if pm.confidence < min_confidence {
                    return None;
                }
                Some(pm)
            })
            .collect())
    }

    /// Reinforces a stored procedure using the configured strategy.
    ///
    /// # Errors
    ///
    /// Returns an error when procedure retrieval or update fails.
    pub fn reinforce(&self, procedure_id: u64, success: bool) -> Result<(), AgentMemoryError> {
        self.reinforce_with_strategy(procedure_id, success, &*self.reinforcement_strategy)
    }

    /// Reinforces a stored procedure using a custom strategy.
    ///
    /// # Errors
    ///
    /// Returns an error when procedure retrieval or update fails.
    #[allow(deprecated)]
    pub fn reinforce_with_strategy(
        &self,
        procedure_id: u64,
        success: bool,
        strategy: &dyn ReinforcementStrategy,
    ) -> Result<(), AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        let points = collection.get(&[procedure_id]);
        let point = points
            .into_iter()
            .flatten()
            .next()
            .ok_or_else(|| AgentMemoryError::NotFound(format!("Procedure {procedure_id}")))?;

        let state = Self::extract_procedure_state(&point)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let context = state.build_reinforcement_context(now);
        let new_confidence = strategy.update_confidence(state.confidence, success, &context);
        let (new_success, new_failure) = if success {
            (state.success_count + 1, state.failure_count)
        } else {
            (state.success_count, state.failure_count + 1)
        };

        let updated_point = Point::new(
            procedure_id,
            point.vector.clone(),
            Some(json!({
                "name": state.name,
                "steps": state.steps,
                "confidence": new_confidence,
                "usage_count": state.usage_count + 1,
                "created_at": state.created_at,
                "last_used_at": now,
                "success_count": new_success,
                "failure_count": new_failure
            })),
        );

        memory_helpers::upsert_points(&collection, vec![updated_point])?;

        Ok(())
    }

    fn extract_procedure_state(point: &Point) -> Result<ProcedureState, AgentMemoryError> {
        let payload = point
            .payload
            .as_ref()
            .ok_or_else(|| AgentMemoryError::CollectionError("Missing payload".to_string()))?;

        Ok(ProcedureState {
            name: payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            steps: payload
                .get("steps")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            confidence: payload
                .get("confidence")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.5) as f32,
            usage_count: payload
                .get("usage_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            created_at: payload
                .get("created_at")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0),
            success_count: payload
                .get("success_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            failure_count: payload
                .get("failure_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        })
    }

    /// Lists all tracked procedures.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access fails.
    #[allow(deprecated)]
    pub fn list_all(&self) -> Result<Vec<ProcedureMatch>, AgentMemoryError> {
        let collection = memory_helpers::get_collection(&self.db, &self.collection_name)?;

        let all_ids: Vec<u64> = self.stored_ids.read().iter().copied().collect();
        let points = collection.get(&all_ids);

        Ok(points
            .into_iter()
            .flatten()
            .filter(|p| !self.ttl.is_expired(p.id))
            .filter_map(|p| extract_procedure_match(&p, 0.0))
            .collect())
    }

    /// Deletes a procedure by id.
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
        )
    }

    /// Serializes all procedures into snapshot bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when collection access or JSON encoding fails.
    pub fn serialize(&self) -> Result<Vec<u8>, AgentMemoryError> {
        memory_helpers::serialize_tracked_points(&self.db, &self.collection_name, &self.stored_ids)
    }

    /// Replaces procedural memory state from serialized snapshot bytes.
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

/// Extracts a `ProcedureMatch` from a point's payload with the given similarity score.
fn extract_procedure_match(point: &Point, score: f32) -> Option<ProcedureMatch> {
    let payload = point.payload.as_ref()?;
    let name = payload.get("name")?.as_str()?.to_string();
    let steps: Vec<String> = payload
        .get("steps")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    #[allow(clippy::cast_possible_truncation)]
    let confidence = payload.get("confidence")?.as_f64()? as f32;

    Some(ProcedureMatch {
        id: point.id,
        name,
        steps,
        confidence,
        score,
    })
}
