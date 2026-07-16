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
fn test_source_for_carries_id_and_handle_without_memory() {
    let source = source_for(99);
    assert_eq!(source.fragment_id, 99);
    assert_eq!(source.handle, "ctx://source/99");
    assert!(source.memory_id.is_none());
}
