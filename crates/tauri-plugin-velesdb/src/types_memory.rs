//! Agent-memory request/response DTOs extracted from `types.rs`.
//!
//! Semantic, episodic and procedural memory commands, plus TTL, eviction,
//! snapshot-versioning and memory-query DTOs. Like every Tauri IPC type they
//! use `camelCase` for JavaScript callers; `types` re-exports them so the
//! `crate::types::*` paths stay stable.

use serde::{Deserialize, Serialize};
use velesdb_core::api_types::{default_top_k, serde_id};

// ============================================================================
// AgentMemory DTOs (EPIC-016 US-003)
// ============================================================================

/// Request to store knowledge in semantic memory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticStoreRequest {
    /// Unique ID for this knowledge fact.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Text content of the knowledge.
    pub content: String,
    /// Embedding vector for the content.
    pub embedding: Vec<f32>,
}

/// Request to query semantic memory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticQueryRequest {
    /// Query embedding vector.
    pub embedding: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Search quality for this query, as [`SearchRequest::quality`] takes it.
    /// Absent keeps the memory collection's own quality.
    #[serde(default)]
    pub quality: Option<String>,
}

/// Result from semantic memory query.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticQueryResult {
    /// Knowledge fact ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Similarity score.
    pub score: f32,
    /// Knowledge content text.
    pub content: String,
}

/// Request to store knowledge in semantic memory with a TTL.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticStoreWithTtlRequest {
    /// Unique ID for this knowledge fact.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Text content of the knowledge.
    pub content: String,
    /// Embedding vector for the content.
    pub embedding: Vec<f32>,
    /// Time-to-live in seconds before the entry expires.
    pub ttl_seconds: u64,
}

/// Request to delete a semantic memory entry by ID.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticDeleteRequest {
    /// Knowledge fact ID to delete.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
}

/// Request to record an episode in episodic memory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecordRequest {
    /// Episode event ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub event_id: u64,
    /// Episode description/content.
    pub content: String,
    /// Timestamp (epoch seconds).
    pub timestamp: i64,
    /// Embedding vector for the episode.
    pub embedding: Vec<f32>,
}

/// Request to query recent episodes.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecentRequest {
    /// Number of recent episodes to return.
    #[serde(default = "default_top_k")]
    pub limit: usize,
    /// Only return episodes since this timestamp (epoch seconds).
    #[serde(default)]
    pub since_timestamp: Option<i64>,
}

/// Result from episodic memory query.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicResult {
    /// Episode ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Episode content.
    pub content: String,
    /// Timestamp (epoch seconds).
    pub timestamp: i64,
}

/// Request to delete an episode by ID.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicDeleteRequest {
    /// Episode event ID to delete.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub event_id: u64,
}

/// Request to recall episodes similar to a query embedding.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicRecallSimilarRequest {
    /// Query embedding vector.
    pub embedding: Vec<f32>,
    /// Number of results to return.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
}

/// Request to fetch episodes older than a timestamp.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicOlderThanRequest {
    /// Upper timestamp bound (epoch seconds); only older events are returned.
    pub timestamp: i64,
    /// Maximum number of results to return.
    #[serde(default = "default_top_k")]
    pub limit: usize,
}

/// Result from a similarity-based episodic recall.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodicSimilarResult {
    /// Episode ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Episode content.
    pub content: String,
    /// Timestamp (epoch seconds).
    pub timestamp: i64,
    /// Similarity score from the vector search.
    pub score: f32,
}

// ============================================================================
// ProceduralMemory DTOs
// ============================================================================

/// Default confidence for procedural learning.
#[must_use]
pub const fn default_confidence() -> f32 {
    1.0
}

/// Request to learn a procedure.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralLearnRequest {
    /// Procedure ID.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub procedure_id: u64,
    /// Procedure name.
    pub name: String,
    /// Steps to perform.
    pub steps: Vec<String>,
    /// Embedding vector for the procedure.
    pub embedding: Vec<f32>,
    /// Confidence level (0.0-1.0). Default: 1.0.
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

/// Request to recall procedures by similarity.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralRecallRequest {
    /// Query embedding vector.
    pub embedding: Vec<f32>,
    /// Number of results.
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    /// Minimum confidence threshold. Default: 0.0 (no filter).
    #[serde(default)]
    pub min_confidence: f32,
}

/// Result from procedural memory recall.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralMatchResult {
    /// Procedure ID.
    #[serde(serialize_with = "serde_id::serialize_id_as_string")]
    pub id: u64,
    /// Procedure name.
    pub name: String,
    /// Steps.
    pub steps: Vec<String>,
    /// Confidence score.
    pub confidence: f32,
    /// Similarity score from vector search.
    pub score: f32,
}

/// Request to reinforce a stored procedure.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralReinforceRequest {
    /// Procedure ID to reinforce.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub procedure_id: u64,
    /// Whether the procedure execution succeeded.
    pub success: bool,
}

/// Request to delete a procedure by ID.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProceduralDeleteRequest {
    /// Procedure ID to delete.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub procedure_id: u64,
}

// ============================================================================
// TTL / eviction / snapshot-versioning DTOs (EPIC-016 parity)
// ============================================================================

/// Which memory subsystem a TTL request targets.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryKindDto {
    /// Semantic (long-term knowledge) memory.
    Semantic,
    /// Episodic (event timeline) memory.
    Episodic,
    /// Procedural (learned patterns) memory.
    Procedural,
}

/// Request to set a TTL on a single memory entry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryTtlRequest {
    /// Target memory subsystem.
    pub kind: MemoryKindDto,
    /// Entry ID to expire.
    #[serde(deserialize_with = "serde_id::deserialize_id_from_string_or_number")]
    pub id: u64,
    /// Time-to-live in seconds.
    pub ttl_seconds: u64,
}

/// Result of an `auto_expire` pass over the persistent memory handle.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpireResultDto {
    /// Semantic entries expired.
    pub semantic_expired: usize,
    /// Episodic entries expired.
    pub episodic_expired: usize,
    /// Procedural entries expired.
    pub procedural_expired: usize,
    /// Episodic entries consolidated into semantic memory.
    pub episodic_consolidated: usize,
    /// Procedural entries evicted for low confidence.
    pub procedural_evicted: usize,
    /// `true` when more old episodes remained than this cycle processed.
    pub consolidation_truncated: bool,
}

impl From<velesdb_core::agent::ExpireResult> for ExpireResultDto {
    fn from(r: velesdb_core::agent::ExpireResult) -> Self {
        Self {
            semantic_expired: r.semantic_expired,
            episodic_expired: r.episodic_expired,
            procedural_expired: r.procedural_expired,
            episodic_consolidated: r.episodic_consolidated,
            procedural_evicted: r.procedural_evicted,
            consolidation_truncated: r.consolidation_truncated,
        }
    }
}

/// Request to evict procedures below a confidence threshold.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvictLowConfidenceRequest {
    /// Minimum confidence; procedures below this are evicted.
    pub min_confidence: f32,
}

/// Request to load a specific memory snapshot version.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSnapshotVersionRequest {
    /// Snapshot version number to restore.
    pub version: u64,
}

/// Request executing a `VelesQL` query against one memory subsystem.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryQueryRequest {
    /// `VelesQL` query string.
    pub sql: String,
    /// Named query parameters.
    #[serde(default)]
    pub params: std::collections::HashMap<String, serde_json::Value>,
}
