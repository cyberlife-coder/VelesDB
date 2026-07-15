//! Tests for `upsert` module — shared upsert-mapping logic.

use super::sharded_mappings::ShardedMappings;
use super::upsert::{rollback_upsert, upsert_mapping, upsert_mapping_batch};

// -------------------------------------------------------------------------
// upsert_mapping_batch tests (issue #375)
// -------------------------------------------------------------------------

#[test]
fn test_upsert_mapping_batch_empty() {
    let mappings = ShardedMappings::new();
    let results = upsert_mapping_batch(&mappings, &[]);
    assert!(results.is_empty());
    assert!(mappings.is_empty());
}

#[test]
fn test_upsert_mapping_batch_all_new() {
    let mappings = ShardedMappings::new();
    let ids = [10, 20, 30];
    let results = upsert_mapping_batch(&mappings, &ids);

    assert_eq!(results.len(), 3);
    for result in &results {
        assert_eq!(result.old_idx, None, "new IDs should have no old index");
    }
    assert_eq!(mappings.len(), 3);
}

#[test]
fn test_upsert_mapping_batch_all_existing_reports_old_indices() {
    let mappings = ShardedMappings::new();

    // Pre-insert IDs
    let mut old_indices = Vec::new();
    for id in [10, 20, 30] {
        old_indices.push(mappings.register(id).expect("register"));
    }

    // Batch upsert same IDs
    let results = upsert_mapping_batch(&mappings, &[10, 20, 30]);

    assert_eq!(results.len(), 3);
    for (result, old_idx) in results.iter().zip(&old_indices) {
        assert_eq!(
            result.old_idx,
            Some(*old_idx),
            "existing IDs should report their previous index"
        );
        assert_ne!(result.idx, *old_idx, "replacement allocates a fresh index");
    }
    // Mapping count unchanged; old indices are now unmapped tombstones.
    assert_eq!(mappings.len(), 3);
    for old_idx in old_indices {
        assert_eq!(mappings.get_id(old_idx), None);
    }
}

#[test]
fn test_upsert_mapping_batch_mixed_new_and_existing() {
    let mappings = ShardedMappings::new();

    // Pre-insert one ID
    let old_idx = mappings.register(20).expect("register");

    let results = upsert_mapping_batch(&mappings, &[10, 20, 30]);

    assert_eq!(results.len(), 3);
    // ID 10: new
    assert_eq!(results[0].old_idx, None);
    // ID 20: replaced
    assert_eq!(results[1].old_idx, Some(old_idx));
    // ID 30: new
    assert_eq!(results[2].old_idx, None);

    assert_eq!(mappings.len(), 3);
    // The replaced index no longer resolves to an ID.
    assert_eq!(mappings.get_id(old_idx), None);
}

#[test]
fn test_upsert_mapping_batch_single_element() {
    let mappings = ShardedMappings::new();

    let results = upsert_mapping_batch(&mappings, &[42]);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].old_idx, None);
    assert_eq!(mappings.len(), 1);
}

// -------------------------------------------------------------------------
// upsert_mapping_batch + rollback integration
// -------------------------------------------------------------------------

#[test]
fn test_upsert_mapping_batch_rollback_restores_old_mappings() {
    let mappings = ShardedMappings::new();

    // Pre-insert ID 10
    let original_idx = mappings.register(10).expect("register");

    // Batch upsert replaces ID 10
    let results = upsert_mapping_batch(&mappings, &[10]);
    assert_eq!(results[0].old_idx, Some(original_idx));
    let new_idx = results[0].idx;
    assert_ne!(new_idx, original_idx);

    // Simulate failure: rollback
    rollback_upsert(&mappings, 10, &results[0]);

    // Old mapping restored
    assert_eq!(mappings.get_idx(10), Some(original_idx));
    assert_eq!(mappings.get_id(original_idx), Some(10));
    // New mapping gone
    assert_eq!(mappings.get_id(new_idx), None);
}

#[test]
fn test_upsert_mapping_batch_rollback_reverse_order() {
    let mappings = ShardedMappings::new();

    // Pre-insert IDs 10 and 20
    let idx_10 = mappings.register(10).expect("register");
    let idx_20 = mappings.register(20).expect("register");

    // Batch upsert replaces both
    let results = upsert_mapping_batch(&mappings, &[10, 20]);

    // Rollback in reverse order (as the production code does)
    for (id, result) in [10_u64, 20].iter().zip(results.iter()).rev() {
        rollback_upsert(&mappings, *id, result);
    }

    // Both original mappings restored
    assert_eq!(mappings.get_idx(10), Some(idx_10));
    assert_eq!(mappings.get_idx(20), Some(idx_20));
}

// -------------------------------------------------------------------------
// Consistency: batch result matches sequential upsert_mapping calls
// -------------------------------------------------------------------------

#[test]
fn test_upsert_mapping_batch_matches_sequential() {
    let ids = [100, 200, 300];

    // Sequential path
    let seq_mappings = ShardedMappings::new();
    let seq_results: Vec<_> = ids
        .iter()
        .map(|&id| upsert_mapping(&seq_mappings, id))
        .collect();

    // Batch path
    let batch_mappings = ShardedMappings::new();
    let batch_results = upsert_mapping_batch(&batch_mappings, &ids);

    // Results should be identical for all-new IDs
    assert_eq!(seq_results.len(), batch_results.len());
    for (seq, batch) in seq_results.iter().zip(batch_results.iter()) {
        assert_eq!(seq.idx, batch.idx, "indices should match");
        assert_eq!(seq.old_idx, batch.old_idx, "old_idx should match");
    }
}
