//! Secondary index Tauri commands.
//!
//! Extracted from commands.rs to keep file NLOC under 500.
#![allow(clippy::missing_errors_doc)]

use crate::error::{CommandError, Error};
use crate::helpers::{require_collection, require_vector_collection};
use crate::state::VelesDbState;
use tauri::{command, AppHandle, Runtime, State};

/// Creates a secondary index on a metadata field for faster filtered search.
#[command]
pub async fn create_index<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: crate::types::CreateIndexRequest,
) -> std::result::Result<(), CommandError> {
    state
        .with_db(|db| {
            let coll = require_vector_collection(&db, &request.collection)?;
            coll.create_index(&request.field_name)
                .map_err(|e| Error::InvalidConfig(e.to_string()))?;
            Ok(())
        })
        .map_err(CommandError::from)
}

/// Drops a secondary index on a metadata field.
#[command]
pub async fn drop_index<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: crate::types::DropIndexRequest,
) -> std::result::Result<bool, CommandError> {
    state
        .with_db(|db| {
            let coll = require_collection(&db, &request.collection)?;
            Ok(coll.drop_secondary_index(&request.field_name))
        })
        .map_err(CommandError::from)
}

/// Lists all secondary indexes on a collection.
#[command]
pub async fn list_indexes<R: Runtime>(
    _app: AppHandle<R>,
    state: State<'_, VelesDbState>,
    request: crate::types::ListIndexesRequest,
) -> std::result::Result<Vec<crate::types::IndexInfoOutput>, CommandError> {
    state
        .with_db(|db| {
            let coll = require_vector_collection(&db, &request.collection)?;
            let indexes = coll.list_indexes();
            Ok(indexes
                .into_iter()
                .map(|info| crate::types::IndexInfoOutput {
                    label: info.label,
                    property: info.property,
                    index_type: info.index_type,
                    cardinality: info.cardinality,
                    memory_bytes: info.memory_bytes,
                })
                .collect())
        })
        .map_err(CommandError::from)
}
