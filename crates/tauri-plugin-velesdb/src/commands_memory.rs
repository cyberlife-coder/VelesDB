//! `AgentMemory` Tauri commands extracted from `commands.rs` (EPIC-016 US-003).
//!
//! Contains semantic store and semantic query commands.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::state::VelesDbState;
use crate::types::{SemanticQueryRequest, SemanticQueryResult, SemanticStoreRequest};
use tauri::{command, AppHandle, Runtime, State};
use velesdb_core::agent::SemanticMemory;

/// Creates a `SemanticMemory` instance, converting agent errors to plugin errors.
fn open_semantic_memory(
    db: std::sync::Arc<velesdb_core::Database>,
    dimension: usize,
) -> std::result::Result<SemanticMemory, Error> {
    SemanticMemory::new_from_db(db, dimension).map_err(|e| Error::InvalidConfig(e.to_string()))
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
