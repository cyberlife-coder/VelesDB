//! Tauri event system for `VelesDB` notifications (EPIC-015 US-004).
//!
//! Provides events to notify the frontend of database changes.
//!
//! # Events
//!
//! - `velesdb://collection-created` - New collection created
//! - `velesdb://collection-deleted` - Collection deleted
//! - `velesdb://collection-updated` - Collection modified (upsert/delete)
//! - `velesdb://operation-progress` - Long operation progress
//! - `velesdb://operation-complete` - Operation completed
//!
//! # Usage (JavaScript)
//!
//! ```javascript
//! import { listen } from '@tauri-apps/api/event';
//!
//! await listen('velesdb://collection-updated', (event) => {
//!   console.log('Collection updated:', event.payload);
//! });
//! ```

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

/// Event names for `VelesDB` notifications.
pub mod event_names {
    /// Emitted when a new collection is created.
    pub const COLLECTION_CREATED: &str = "velesdb://collection-created";
    /// Emitted when a collection is deleted.
    pub const COLLECTION_DELETED: &str = "velesdb://collection-deleted";
    /// Emitted when a collection is updated (upsert/delete points).
    pub const COLLECTION_UPDATED: &str = "velesdb://collection-updated";
    /// Emitted during long operations to report progress.
    pub const OPERATION_PROGRESS: &str = "velesdb://operation-progress";
    /// Emitted when an operation completes.
    pub const OPERATION_COMPLETE: &str = "velesdb://operation-complete";
}

/// Payload for collection events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionEventPayload {
    /// Collection name.
    pub collection: String,
    /// Operation type (created, deleted, upsert, `delete_points`).
    pub operation: String,
    /// Number of affected items (for upsert/delete).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

/// Payload for operation progress events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressEventPayload {
    /// Operation ID (for tracking).
    pub operation_id: String,
    /// Current progress (0-100).
    pub progress: u8,
    /// Total items to process.
    pub total: usize,
    /// Items processed so far.
    pub processed: usize,
    /// Optional message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Payload for operation complete events.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompleteEventPayload {
    /// Operation ID.
    pub operation_id: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// Optional error message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Duration in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Emits a collection created event.
pub fn emit_collection_created<R: Runtime>(app: &AppHandle<R>, collection: &str) {
    let payload = CollectionEventPayload {
        collection: collection.to_string(),
        operation: "created".to_string(),
        count: None,
    };
    let _ = app.emit(event_names::COLLECTION_CREATED, payload);
}

/// Emits a collection deleted event.
pub fn emit_collection_deleted<R: Runtime>(app: &AppHandle<R>, collection: &str) {
    let payload = CollectionEventPayload {
        collection: collection.to_string(),
        operation: "deleted".to_string(),
        count: None,
    };
    let _ = app.emit(event_names::COLLECTION_DELETED, payload);
}

/// Emits a collection updated event (after upsert or delete).
pub fn emit_collection_updated<R: Runtime>(
    app: &AppHandle<R>,
    collection: &str,
    operation: &str,
    count: usize,
) {
    let payload = CollectionEventPayload {
        collection: collection.to_string(),
        operation: operation.to_string(),
        count: Some(count),
    };
    let _ = app.emit(event_names::COLLECTION_UPDATED, payload);
}

/// Emits an operation progress event.
pub fn emit_progress<R: Runtime>(
    app: &AppHandle<R>,
    operation_id: &str,
    processed: usize,
    total: usize,
    message: Option<&str>,
) {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let progress = if total > 0 {
        ((processed as f64 / total as f64) * 100.0).min(100.0) as u8
    } else {
        0
    };
    let payload = ProgressEventPayload {
        operation_id: operation_id.to_string(),
        progress,
        total,
        processed,
        message: message.map(String::from),
    };
    let _ = app.emit(event_names::OPERATION_PROGRESS, payload);
}

/// Emits an operation complete event.
pub fn emit_complete<R: Runtime>(
    app: &AppHandle<R>,
    operation_id: &str,
    success: bool,
    error: Option<&str>,
    duration_ms: Option<u64>,
) {
    let payload = CompleteEventPayload {
        operation_id: operation_id.to_string(),
        success,
        error: error.map(String::from),
        duration_ms,
    };
    let _ = app.emit(event_names::OPERATION_COMPLETE, payload);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collection_event_payload_serialization() {
        let payload = CollectionEventPayload {
            collection: "test".to_string(),
            operation: "created".to_string(),
            count: None,
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"collection\":\"test\""));
        assert!(json.contains("\"operation\":\"created\""));
        assert!(!json.contains("count")); // skip_serializing_if
    }

    #[test]
    fn test_progress_event_payload_serialization() {
        let payload = ProgressEventPayload {
            operation_id: "op-123".to_string(),
            progress: 50,
            total: 100,
            processed: 50,
            message: Some("Processing...".to_string()),
        };
        let json = serde_json::to_string(&payload).unwrap();
        assert!(json.contains("\"operationId\":\"op-123\""));
        assert!(json.contains("\"progress\":50"));
    }
}
