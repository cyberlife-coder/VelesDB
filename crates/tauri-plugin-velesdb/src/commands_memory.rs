//! `AgentMemory` Tauri commands extracted from `commands.rs` (EPIC-016 US-003).
//!
//! Contains semantic, episodic, and procedural memory commands.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::state::VelesDbState;
use crate::types::{
    EpisodicRecentRequest, EpisodicRecordRequest, EpisodicResult, ProceduralLearnRequest,
    ProceduralMatchResult, ProceduralRecallRequest, SemanticQueryRequest, SemanticQueryResult,
    SemanticStoreRequest,
};
use tauri::{command, AppHandle, Runtime, State};
use velesdb_core::agent::{EpisodicMemory, ProceduralMemory, SemanticMemory};

/// Creates a `SemanticMemory` instance, converting agent errors to plugin errors.
fn open_semantic_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<SemanticMemory, Error> {
    SemanticMemory::new_from_db(db, dimension).map_err(|e| Error::InvalidConfig(e.to_string()))
}

/// Creates an `EpisodicMemory` instance, converting agent errors to plugin errors.
fn open_episodic_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<EpisodicMemory, Error> {
    EpisodicMemory::new_from_db(db, dimension).map_err(|e| Error::InvalidConfig(e.to_string()))
}

/// Creates a `ProceduralMemory` instance, converting agent errors to plugin errors.
fn open_procedural_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<ProceduralMemory, Error> {
    ProceduralMemory::new_from_db(db, dimension).map_err(|e| Error::InvalidConfig(e.to_string()))
}

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
            memory
                .store(request.id, &request.content, &request.embedding)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
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
            let results = memory
                .query(&request.embedding, request.top_k)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(results
                .into_iter()
                .map(|(id, score, content)| SemanticQueryResult { id, score, content })
                .collect())
        })
        .map_err(CommandError::from)
}

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
            memory
                .record(
                    request.event_id,
                    &request.content,
                    request.timestamp,
                    Some(&request.embedding),
                )
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
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
            let results = memory
                .recent(request.limit, request.since_timestamp)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
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
            memory
                .learn(
                    request.procedure_id,
                    &request.name,
                    &request.steps,
                    Some(&request.embedding),
                    request.confidence,
                )
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
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
            let results = memory
                .recall(&request.embedding, request.top_k, request.min_confidence)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(results
                .into_iter()
                .map(|m| ProceduralMatchResult {
                    id: m.id,
                    name: m.name,
                    steps: m.steps,
                    confidence: m.confidence,
                    score: m.score,
                })
                .collect())
        })
        .map_err(CommandError::from)
}
