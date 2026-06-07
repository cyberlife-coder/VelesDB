//! `AgentMemory` Tauri commands extracted from `commands.rs` (EPIC-016 US-003).
//!
//! Contains semantic, episodic, and procedural memory commands, plus snapshot
//! serialize/deserialize commands. Agent errors keep their typed variants
//! (`DimensionMismatch`, `NotFound`) through the `From<AgentMemoryError>`
//! conversion in [`crate::error`].
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::state::VelesDbState;
use crate::types::{
    EpisodicDeleteRequest, EpisodicOlderThanRequest, EpisodicRecallSimilarRequest,
    EpisodicRecentRequest, EpisodicRecordRequest, EpisodicResult, EpisodicSimilarResult,
    MemoryRestoreRequest, MemorySnapshotRequest, ProceduralDeleteRequest, ProceduralLearnRequest,
    ProceduralMatchResult, ProceduralRecallRequest, ProceduralReinforceRequest,
    SemanticDeleteRequest, SemanticQueryRequest, SemanticQueryResult, SemanticStoreRequest,
    SemanticStoreWithTtlRequest,
};
use tauri::{command, AppHandle, Runtime, State};
use velesdb_core::agent::{EpisodicMemory, ProceduralMemory, SemanticMemory};

/// Creates a `SemanticMemory` instance, preserving typed agent errors.
fn open_semantic_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<SemanticMemory, Error> {
    Ok(SemanticMemory::new_from_db(db, dimension)?)
}

/// Creates an `EpisodicMemory` instance, preserving typed agent errors.
fn open_episodic_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<EpisodicMemory, Error> {
    Ok(EpisodicMemory::new_from_db(db, dimension)?)
}

/// Creates a `ProceduralMemory` instance, preserving typed agent errors.
fn open_procedural_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<ProceduralMemory, Error> {
    Ok(ProceduralMemory::new_from_db(db, dimension)?)
}

// ============================================================================
// Semantic memory commands
// ============================================================================

/// Stores a knowledge fact in semantic memory.
#[command]
pub async fn semantic_store<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SemanticStoreRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, request.embedding.len())?;
            memory.store(request.id, &request.content, &request.embedding)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Stores a knowledge fact in semantic memory with a TTL.
#[command]
pub async fn semantic_store_with_ttl<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SemanticStoreWithTtlRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, request.embedding.len())?;
            memory.store_with_ttl(
                request.id,
                &request.content,
                &request.embedding,
                request.ttl_seconds,
            )?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Queries semantic memory by similarity search.
#[command]
pub async fn semantic_query<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SemanticQueryRequest,
) -> std::result::Result<Vec<SemanticQueryResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, request.embedding.len())?;
            let results = memory.query(&request.embedding, request.top_k)?;
            Ok(results
                .into_iter()
                .map(|(id, score, content)| SemanticQueryResult { id, score, content })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Deletes a knowledge fact from semantic memory.
#[command]
pub async fn semantic_delete<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: SemanticDeleteRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, crate::types::default_dimension())?;
            memory.delete(request.id)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Returns the embedding dimension of semantic memory.
#[command]
pub async fn semantic_dimension<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<usize, CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, crate::types::default_dimension())?;
            Ok(memory.dimension())
        })
        .map_err(CommandError::from)
}

/// Serializes semantic memory to snapshot bytes.
#[command]
pub async fn semantic_serialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemorySnapshotRequest,
) -> std::result::Result<Vec<u8>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, request.dimension)?;
            Ok(memory.serialize()?)
        })
        .map_err(CommandError::from)
}

/// Restores semantic memory from snapshot bytes.
#[command]
pub async fn semantic_deserialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryRestoreRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_semantic_memory(db, request.dimension)?;
            memory.deserialize(&request.data)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

// ============================================================================
// Episodic memory commands
// ============================================================================

/// Records an episode in episodic memory.
#[command]
pub async fn episodic_record<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EpisodicRecordRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, request.embedding.len())?;
            memory.record(
                request.event_id,
                &request.content,
                request.timestamp,
                Some(&request.embedding),
            )?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Queries recent episodes from episodic memory.
#[command]
pub async fn episodic_recent<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EpisodicRecentRequest,
) -> std::result::Result<Vec<EpisodicResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, crate::types::default_dimension())?;
            let results = memory.recent(request.limit, request.since_timestamp)?;
            Ok(results
                .into_iter()
                .map(|(id, content, timestamp)| EpisodicResult {
                    id,
                    content,
                    timestamp,
                })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Recalls episodes similar to a query embedding.
#[command]
pub async fn episodic_recall_similar<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EpisodicRecallSimilarRequest,
) -> std::result::Result<Vec<EpisodicSimilarResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, request.embedding.len())?;
            let results = memory.recall_similar(&request.embedding, request.top_k)?;
            Ok(results
                .into_iter()
                .map(|(id, content, timestamp, score)| EpisodicSimilarResult {
                    id,
                    content,
                    timestamp,
                    score,
                })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Returns episodes older than a timestamp.
#[command]
pub async fn episodic_older_than<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EpisodicOlderThanRequest,
) -> std::result::Result<Vec<EpisodicResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, crate::types::default_dimension())?;
            let results = memory.older_than(request.timestamp, request.limit)?;
            Ok(results
                .into_iter()
                .map(|(id, content, timestamp)| EpisodicResult {
                    id,
                    content,
                    timestamp,
                })
                .collect())
        })
        .map_err(CommandError::from)
}

/// Deletes an episode from episodic memory.
#[command]
pub async fn episodic_delete<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EpisodicDeleteRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, crate::types::default_dimension())?;
            memory.delete(request.event_id)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Serializes episodic memory to snapshot bytes.
#[command]
pub async fn episodic_serialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemorySnapshotRequest,
) -> std::result::Result<Vec<u8>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, request.dimension)?;
            Ok(memory.serialize()?)
        })
        .map_err(CommandError::from)
}

/// Restores episodic memory from snapshot bytes.
#[command]
pub async fn episodic_deserialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryRestoreRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_episodic_memory(db, request.dimension)?;
            memory.deserialize(&request.data)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

// ============================================================================
// Procedural memory commands
// ============================================================================

/// Learns a procedure in procedural memory.
#[command]
pub async fn procedural_learn<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: ProceduralLearnRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, request.embedding.len())?;
            memory.learn(
                request.procedure_id,
                &request.name,
                &request.steps,
                Some(&request.embedding),
                request.confidence,
            )?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Recalls procedures by similarity from procedural memory.
#[command]
pub async fn procedural_recall<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: ProceduralRecallRequest,
) -> std::result::Result<Vec<ProceduralMatchResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, request.embedding.len())?;
            let results =
                memory.recall(&request.embedding, request.top_k, request.min_confidence)?;
            Ok(results.into_iter().map(to_match_result).collect())
        })
        .map_err(CommandError::from)
}

/// Reinforces a stored procedure.
#[command]
pub async fn procedural_reinforce<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: ProceduralReinforceRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, crate::types::default_dimension())?;
            memory.reinforce(request.procedure_id, request.success)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Lists all stored procedures.
#[command]
pub async fn procedural_list_all<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<ProceduralMatchResult>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, crate::types::default_dimension())?;
            let results = memory.list_all()?;
            Ok(results.into_iter().map(to_match_result).collect())
        })
        .map_err(CommandError::from)
}

/// Deletes a procedure from procedural memory.
#[command]
pub async fn procedural_delete<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: ProceduralDeleteRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, crate::types::default_dimension())?;
            memory.delete(request.procedure_id)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Serializes procedural memory to snapshot bytes.
#[command]
pub async fn procedural_serialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemorySnapshotRequest,
) -> std::result::Result<Vec<u8>, CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, request.dimension)?;
            Ok(memory.serialize()?)
        })
        .map_err(CommandError::from)
}

/// Restores procedural memory from snapshot bytes.
#[command]
pub async fn procedural_deserialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryRestoreRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let memory = open_procedural_memory(db, request.dimension)?;
            memory.deserialize(&request.data)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Maps a core `ProcedureMatch` to the Tauri `ProceduralMatchResult` DTO.
fn to_match_result(m: velesdb_core::agent::ProcedureMatch) -> ProceduralMatchResult {
    ProceduralMatchResult {
        id: m.id,
        name: m.name,
        steps: m.steps,
        confidence: m.confidence,
        score: m.score,
    }
}
