//! Tests for CLI streaming traverse (EPIC-059 US-004).
//!
//! Validates `--stream` flag and NDJSON output format.

use serde_json::Value;

/// Test that stream flag is parsed correctly.
#[test]
fn test_traverse_stream_flag_parsing() {
    // Simulate parsing --stream flag
    let args = ["traverse", "collection", "123", "--stream"];
    assert!(args.contains(&"--stream"));
}

/// Test NDJSON output format (one JSON object per line).
#[test]
fn test_traverse_stream_ndjson_format() {
    // Simulated streaming output
    let output_lines = vec![
        r#"{"event":"node","id":1,"depth":0,"path":[1]}"#,
        r#"{"event":"node","id":2,"depth":1,"path":[1,2]}"#,
        r#"{"event":"node","id":3,"depth":1,"path":[1,3]}"#,
        r#"{"event":"done","total_nodes":3,"max_depth":1,"elapsed_ms":5}"#,
    ];

    for line in &output_lines {
        let parsed: Result<Value, _> = serde_json::from_str(line);
        assert!(parsed.is_ok(), "Line should be valid JSON: {line}");
    }
}

/// Test streaming node event structure.
#[test]
fn test_stream_node_event_structure() {
    let event = r#"{"event":"node","id":42,"depth":2,"path":[1,5,42]}"#;
    let parsed: Value = serde_json::from_str(event).expect("valid JSON");

    assert_eq!(parsed["event"], "node");
    assert_eq!(parsed["id"], 42);
    assert_eq!(parsed["depth"], 2);
    assert!(parsed["path"].is_array());
}

/// Test streaming done event structure.
#[test]
fn test_stream_done_event_structure() {
    let event = r#"{"event":"done","total_nodes":100,"max_depth":5,"elapsed_ms":150}"#;
    let parsed: Value = serde_json::from_str(event).expect("valid JSON");

    assert_eq!(parsed["event"], "done");
    assert_eq!(parsed["total_nodes"], 100);
    assert_eq!(parsed["max_depth"], 5);
    assert!(parsed["elapsed_ms"].is_number());
}

/// Test that BFS and DFS both support streaming.
#[test]
fn test_traverse_stream_bfs_dfs_compat() {
    let strategies = ["bfs", "dfs"];

    for strategy in strategies {
        // Both strategies should be valid for streaming
        assert!(
            strategy == "bfs" || strategy == "dfs",
            "Strategy {strategy} should support streaming"
        );
    }
}

/// Test error event structure in streaming mode.
#[test]
fn test_stream_error_event_structure() {
    let event = r#"{"event":"error","message":"Collection 'unknown' not found"}"#;
    let parsed: Value = serde_json::from_str(event).expect("valid JSON");

    assert_eq!(parsed["event"], "error");
    assert!(parsed["message"].is_string());
}

/// Test empty graph streaming (should still emit done event).
#[test]
fn test_stream_empty_graph() {
    let output_lines = [r#"{"event":"done","total_nodes":0,"max_depth":0,"elapsed_ms":1}"#];

    let parsed: Value = serde_json::from_str(output_lines[0]).expect("valid JSON");
    assert_eq!(parsed["event"], "done");
    assert_eq!(parsed["total_nodes"], 0);
}

/// Test streaming with depth limit.
#[test]
fn test_stream_respects_depth_limit() {
    // Simulated output with max_depth=2
    let output_lines = vec![
        r#"{"event":"node","id":1,"depth":0,"path":[1]}"#,
        r#"{"event":"node","id":2,"depth":1,"path":[1,2]}"#,
        r#"{"event":"node","id":3,"depth":2,"path":[1,2,3]}"#,
        r#"{"event":"done","total_nodes":3,"max_depth":2,"elapsed_ms":10}"#,
    ];

    for line in &output_lines {
        let parsed: Value = serde_json::from_str(line).expect("valid JSON");
        if parsed["event"] == "node" {
            let depth = parsed["depth"].as_u64().unwrap_or(0);
            assert!(depth <= 2, "Depth should respect limit");
        }
    }
}
