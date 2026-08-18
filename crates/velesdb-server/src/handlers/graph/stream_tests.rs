use super::*;

#[test]
fn test_stream_node_event_serialize() {
    let event = StreamNodeEvent {
        id: 123,
        depth: 2,
        path: vec![1, 2],
    };
    let json = serde_json::to_string(&event).expect("should serialize");
    assert!(json.contains("123"));
    assert!(json.contains("\"depth\":2"));
}

#[test]
fn test_stream_done_event_serialize() {
    let event = StreamDoneEvent {
        total_nodes: 100,
        max_depth_reached: 5,
        elapsed_ms: 150,
    };
    let json = serde_json::to_string(&event).expect("should serialize");
    assert!(json.contains("100"));
    assert!(json.contains("max_depth_reached"));
}

#[test]
fn test_stream_error_event_serialize() {
    let event = StreamErrorEvent {
        error: "Collection not found".to_string(),
    };
    let json = serde_json::to_string(&event).expect("should serialize");
    assert!(json.contains("Collection not found"));
}

#[test]
fn test_build_error_events_returns_single_error() {
    let events = build_error_events("test error".to_string());
    assert_eq!(events.len(), 1);
}

#[test]
fn test_elapsed_ms_returns_reasonable_value() {
    let start = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let ms = elapsed_ms(start);
    assert!(ms >= 5, "elapsed should be at least 5ms, got {ms}");
}
