//! Unit tests for source handles.

use super::*;

#[test]
fn test_handle_for_uses_the_ctx_source_scheme() {
    assert_eq!(handle_for(42), "ctx://source/42");
}

#[test]
fn test_handle_for_is_deterministic() {
    assert_eq!(handle_for(7), handle_for(7));
}

#[test]
fn test_source_for_carries_id_and_content_addressed_handle() {
    let source = source_for(99, 12345);
    assert_eq!(source.fragment_id, 99);
    assert_eq!(
        source.handle, "ctx://source/12345",
        "the handle must be minted from the content hash, not the caller id"
    );
    assert!(source.memory_id.is_none());
}

#[test]
fn test_parse_handle_round_trips_handle_for() {
    assert_eq!(parse_handle(&handle_for(42)), Some(42));
}

#[test]
fn test_parse_handle_rejects_malformed_input() {
    for bad in [
        "",
        "not-a-handle",
        "ctx://source/",
        "ctx://source/xyz",
        "ctx://other/1",
    ] {
        assert_eq!(parse_handle(bad), None, "input: {bad}");
    }
}
