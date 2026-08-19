use super::*;

#[test]
fn test_collection_event_payload_serialization() {
    let payload = CollectionEventPayload {
        collection: "test".to_string(),
        operation: "created".to_string(),
        count: None,
    };
    let json = match serde_json::to_string(&payload) {
        Ok(json) => json,
        Err(err) => panic!("payload serialization should succeed: {err}"),
    };
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
    let json = match serde_json::to_string(&payload) {
        Ok(json) => json,
        Err(err) => panic!("payload serialization should succeed: {err}"),
    };
    assert!(json.contains("\"operationId\":\"op-123\""));
    assert!(json.contains("\"progress\":50"));
}
