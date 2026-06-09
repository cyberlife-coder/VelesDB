//! `AgentMemory` Tauri commands extracted from `commands.rs` (EPIC-016 US-003).
//!
//! Contains semantic, episodic, and procedural memory commands, plus snapshot
//! serialize/deserialize commands and the TTL / eviction / snapshot-versioning
//! / `VelesQL` parity commands. Agent errors keep their typed variants
//! (`DimensionMismatch`, `NotFound`) through the `From<AgentMemoryError>`
//! conversion in [`crate::error`].
//!
//! All commands route through the single persistent [`AgentMemory`] handle held
//! in [`VelesDbState`] (see `state::with_memory`), so the in-memory TTL registry,
//! temporal index, and snapshot manager survive across invocations. Re-opening a
//! fresh memory per command silently dropped the TTL registry, which is why TTL,
//! auto-expire, and snapshot versioning previously had no effect.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::state::VelesDbState;
use crate::types::{
    EpisodicDeleteRequest, EpisodicOlderThanRequest, EpisodicRecallSimilarRequest,
    EpisodicRecentRequest, EpisodicRecordRequest, EpisodicResult, EpisodicSimilarResult,
    EvictLowConfidenceRequest, ExpireResultDto, HybridResult, LoadSnapshotVersionRequest,
    MemoryKindDto, MemoryQueryRequest, MemoryTtlRequest, ProceduralDeleteRequest,
    ProceduralLearnRequest, ProceduralMatchResult, ProceduralRecallRequest,
    ProceduralReinforceRequest, SemanticDeleteRequest, SemanticQueryRequest, SemanticQueryResult,
    SemanticStoreRequest, SemanticStoreWithTtlRequest,
};
use tauri::{command, AppHandle, Runtime, State};

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
        .with_memory(|mem| {
            mem.semantic()
                .store(request.id, &request.content, &request.embedding)?;
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
        .with_memory(|mem| {
            mem.semantic().store_with_ttl(
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
        .with_memory(|mem| {
            let results = mem.semantic().query(&request.embedding, request.top_k)?;
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
        .with_memory(|mem| {
            mem.semantic().delete(request.id)?;
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
        .with_memory(|mem| Ok(mem.semantic().dimension()))
        .map_err(CommandError::from)
}

/// Serializes semantic memory to snapshot bytes.
#[command]
pub async fn semantic_serialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<u8>, CommandError> {
    state
        .with_memory(|mem| Ok(mem.semantic().serialize()?))
        .map_err(CommandError::from)
}

/// Restores semantic memory from snapshot bytes.
#[command]
pub async fn semantic_deserialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    data: Vec<u8>,
) -> std::result::Result<(), CommandError> {
    state
        .with_memory(|mem| {
            mem.semantic().deserialize(&data)?;
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
        .with_memory(|mem| {
            mem.episodic().record(
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
        .with_memory(|mem| {
            let results = mem
                .episodic()
                .recent(request.limit, request.since_timestamp)?;
            Ok(results.into_iter().map(to_episodic_result).collect())
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
        .with_memory(|mem| {
            let results = mem
                .episodic()
                .recall_similar(&request.embedding, request.top_k)?;
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
        .with_memory(|mem| {
            let results = mem
                .episodic()
                .older_than(request.timestamp, request.limit)?;
            Ok(results.into_iter().map(to_episodic_result).collect())
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
        .with_memory(|mem| {
            mem.episodic().delete(request.event_id)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Serializes episodic memory to snapshot bytes.
#[command]
pub async fn episodic_serialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<u8>, CommandError> {
    state
        .with_memory(|mem| Ok(mem.episodic().serialize()?))
        .map_err(CommandError::from)
}

/// Restores episodic memory from snapshot bytes.
#[command]
pub async fn episodic_deserialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    data: Vec<u8>,
) -> std::result::Result<(), CommandError> {
    state
        .with_memory(|mem| {
            mem.episodic().deserialize(&data)?;
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
        .with_memory(|mem| {
            mem.procedural().learn(
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
        .with_memory(|mem| {
            let results = mem.procedural().recall(
                &request.embedding,
                request.top_k,
                request.min_confidence,
            )?;
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
        .with_memory(|mem| {
            mem.procedural()
                .reinforce(request.procedure_id, request.success)?;
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
        .with_memory(|mem| {
            let results = mem.procedural().list_all()?;
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
        .with_memory(|mem| {
            mem.procedural().delete(request.procedure_id)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Serializes procedural memory to snapshot bytes.
#[command]
pub async fn procedural_serialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<u8>, CommandError> {
    state
        .with_memory(|mem| Ok(mem.procedural().serialize()?))
        .map_err(CommandError::from)
}

/// Restores procedural memory from snapshot bytes.
#[command]
pub async fn procedural_deserialize<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    data: Vec<u8>,
) -> std::result::Result<(), CommandError> {
    state
        .with_memory(|mem| {
            mem.procedural().deserialize(&data)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

// ============================================================================
// TTL / eviction commands (EPIC-016 parity)
// ============================================================================

/// Sets a TTL on a single memory entry in the persistent registry.
#[command]
pub async fn memory_set_ttl<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryTtlRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_memory(|mem| {
            match request.kind {
                MemoryKindDto::Semantic => mem.set_semantic_ttl(request.id, request.ttl_seconds),
                MemoryKindDto::Episodic => mem.set_episodic_ttl(request.id, request.ttl_seconds),
                MemoryKindDto::Procedural => {
                    mem.set_procedural_ttl(request.id, request.ttl_seconds);
                }
            }
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Expires entries past their TTL and applies eviction/consolidation policies.
#[command]
pub async fn memory_auto_expire<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<ExpireResultDto, CommandError> {
    state
        .with_memory(|mem| Ok(ExpireResultDto::from(mem.auto_expire()?)))
        .map_err(CommandError::from)
}

/// Evicts procedures with confidence below the supplied threshold.
#[command]
pub async fn memory_evict_low_confidence<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: EvictLowConfidenceRequest,
) -> std::result::Result<usize, CommandError> {
    state
        .with_memory(|mem| Ok(mem.evict_low_confidence_procedures(request.min_confidence)?))
        .map_err(CommandError::from)
}

// ============================================================================
// Snapshot-versioning commands (EPIC-016 parity)
// ============================================================================

/// Creates a versioned snapshot of the full memory state. Returns the version.
#[command]
pub async fn memory_snapshot<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<u64, CommandError> {
    state
        .with_memory(|mem| Ok(mem.snapshot()?))
        .map_err(CommandError::from)
}

/// Restores the most recent versioned snapshot. Returns its version.
#[command]
pub async fn memory_load_latest_snapshot<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<u64, CommandError> {
    state
        .with_memory(|mem| Ok(mem.load_latest_snapshot()?))
        .map_err(CommandError::from)
}

/// Restores a specific versioned snapshot.
#[command]
pub async fn memory_load_snapshot_version<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: LoadSnapshotVersionRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_memory(|mem| {
            mem.load_snapshot_version(request.version)?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Lists all available snapshot version numbers.
#[command]
pub async fn memory_list_snapshot_versions<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
) -> std::result::Result<Vec<u64>, CommandError> {
    state
        .with_memory(|mem| Ok(mem.list_snapshot_versions()?))
        .map_err(CommandError::from)
}

// ============================================================================
// VelesQL bridge commands (EPIC-016 parity)
// ============================================================================

/// Executes a `VelesQL` query against semantic memory.
#[command]
pub async fn memory_query_semantic<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryQueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    run_memory_query(&state, |mem| {
        mem.query_semantic(&request.sql, &request.params)
    })
}

/// Executes a `VelesQL` query against episodic memory.
#[command]
pub async fn memory_query_episodic<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryQueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    run_memory_query(&state, |mem| {
        mem.query_episodic(&request.sql, &request.params)
    })
}

/// Executes a `VelesQL` query against procedural memory.
#[command]
pub async fn memory_query_procedural<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: MemoryQueryRequest,
) -> std::result::Result<Vec<HybridResult>, CommandError> {
    run_memory_query(&state, |mem| {
        mem.query_procedural(&request.sql, &request.params)
    })
}

/// Runs a memory `VelesQL` query and maps the rows into `HybridResult` DTOs.
///
/// RF-DEDUP: shared by the semantic / episodic / procedural bridge commands.
fn run_memory_query<F>(
    state: &VelesDbState,
    query: F,
) -> std::result::Result<Vec<HybridResult>, CommandError>
where
    F: FnOnce(
        &velesdb_core::agent::AgentMemory,
    ) -> std::result::Result<
        Vec<velesdb_core::SearchResult>,
        velesdb_core::agent::AgentMemoryError,
    >,
{
    state
        .with_memory(|mem| {
            let rows = query(mem).map_err(Error::from)?;
            Ok(rows
                .iter()
                .map(crate::commands_query::search_result_to_hybrid)
                .collect())
        })
        .map_err(CommandError::from)
}

// ============================================================================
// Mapping helpers
// ============================================================================

/// Maps a core `(id, content, timestamp)` tuple to the `EpisodicResult` DTO.
fn to_episodic_result((id, content, timestamp): (u64, String, i64)) -> EpisodicResult {
    EpisodicResult {
        id,
        content,
        timestamp,
    }
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
