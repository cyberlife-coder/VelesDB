//! Tests for `mappings` module

use super::mappings::*;
use std::collections::HashMap;

#[test]
fn test_mappings_new_is_empty() {
    let mappings = HnswMappings::new();
    assert!(mappings.is_empty());
    assert_eq!(mappings.len(), 0);
}

#[test]
fn test_mappings_register_returns_index() {
    let mut mappings = HnswMappings::new();
    let idx = mappings.register(42);
    assert_eq!(idx, Some(0));
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_mappings_register_increments_index() {
    let mut mappings = HnswMappings::new();
    assert_eq!(mappings.register(1), Some(0));
    assert_eq!(mappings.register(2), Some(1));
    assert_eq!(mappings.register(3), Some(2));
}

#[test]
fn test_mappings_register_duplicate_returns_none() {
    let mut mappings = HnswMappings::new();
    mappings.register(42);
    assert_eq!(mappings.register(42), None);
    assert_eq!(mappings.len(), 1);
}

#[test]
fn test_mappings_get_idx() {
    let mut mappings = HnswMappings::new();
    mappings.register(42);
    assert_eq!(mappings.get_idx(42), Some(0));
    assert_eq!(mappings.get_idx(999), None);
}

#[test]
fn test_mappings_get_id() {
    let mut mappings = HnswMappings::new();
    mappings.register(42);
    assert_eq!(mappings.get_id(0), Some(42));
    assert_eq!(mappings.get_id(999), None);
}

#[test]
fn test_mappings_remove() {
    let mut mappings = HnswMappings::new();
    mappings.register(42);
    assert_eq!(mappings.remove(42), Some(0));
    assert!(mappings.is_empty());
    assert_eq!(mappings.get_idx(42), None);
}

#[test]
fn test_mappings_remove_nonexistent() {
    let mut mappings = HnswMappings::new();
    assert_eq!(mappings.remove(999), None);
}

#[test]
fn test_mappings_from_parts() {
    let mut id_to_idx = HashMap::new();
    let mut idx_to_id = HashMap::new();
    id_to_idx.insert(42, 0);
    idx_to_id.insert(0, 42);

    let mappings = HnswMappings::from_parts(id_to_idx, idx_to_id, 1);
    assert_eq!(mappings.len(), 1);
    assert_eq!(mappings.get_idx(42), Some(0));
}
