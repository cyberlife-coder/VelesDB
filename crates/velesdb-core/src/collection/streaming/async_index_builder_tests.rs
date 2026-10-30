//! Tests for `AsyncIndexBuilder`.

use super::async_index_builder::{AsyncIndexBuilder, AsyncIndexBuilderConfig};
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswIndex;

fn default_config() -> AsyncIndexBuilderConfig {
    AsyncIndexBuilderConfig {
        merge_threshold: 100,
        segment_count: Some(2),
    }
}

fn make_index(dim: usize) -> HnswIndex {
    HnswIndex::new(dim, DistanceMetric::Cosine).expect("test index creation")
}

/// `count` vectors to enqueue, id `i` carrying the `i % dim`-th basis vector.
///
/// Iterating as `usize` keeps the index arithmetic lossless; the id is a
/// `u64`, which `usize` cannot overflow on any supported target.
fn basis_vectors(count: usize, dim: usize) -> Vec<(u64, Vec<f32>)> {
    (0..count)
        .map(|i| {
            let mut v = vec![0.0_f32; dim];
            v[i % dim] = 1.0;
            (u64::try_from(i).expect("test: an id fits a u64"), v)
        })
        .collect()
}

#[test]
fn test_new_creates_empty_builder() {
    let builder = AsyncIndexBuilder::new(default_config());
    assert_eq!(builder.pending_len(), 0);
    assert!(builder.pending_ids().is_empty());
}

#[test]
fn test_enqueue_adds_vectors() {
    let builder = AsyncIndexBuilder::new(default_config());
    let vectors = vec![(1_u64, vec![1.0_f32, 0.0, 0.0])];
    let triggered = builder.enqueue(vectors);
    assert!(!triggered);
    assert_eq!(builder.pending_len(), 1);
}

#[test]
fn test_enqueue_returns_true_at_threshold() {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 3,
        segment_count: Some(1),
    };
    let builder = AsyncIndexBuilder::new(config);

    assert!(!builder.enqueue(vec![(1, vec![1.0])]));
    assert!(!builder.enqueue(vec![(2, vec![2.0])]));
    assert!(builder.enqueue(vec![(3, vec![3.0])]));
}

/// Each pending id once, sorted: an id queued twice, and an id both placed
/// and buffered, are one pending point.
#[test]
fn pending_ids_names_each_waiting_id_once() {
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue_placed([7, 3, 7]);
    builder.enqueue_placed([3]);
    builder.enqueue(vec![(9, vec![1.0]), (3, vec![1.0]), (9, vec![2.0])]);

    let ids = builder.pending_ids();
    assert_eq!(ids.placed, vec![3, 7]);
    assert_eq!(ids.buffered, vec![9], "3 is placed already, 9 is once");
}

#[test]
fn test_flush_sync_indexes_vectors() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());

    builder.enqueue(basis_vectors(20, dim));
    assert_eq!(builder.pending_len(), 20);

    let indexed = builder.flush_sync(&index).unwrap();
    assert_eq!(indexed, 20);
    assert_eq!(builder.pending_len(), 0);
    assert_eq!(index.len(), 20);
}

#[test]
fn test_flush_sync_empty_buffer() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    let indexed = builder.flush_sync(&index).unwrap();
    assert_eq!(indexed, 0);
}

/// A flush that finds another in progress indexes nothing and leaves both
/// queues — and so what `pending_ids` names — as they were.
#[test]
fn flush_sync_skips_while_another_flush_runs() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue(basis_vectors(5, dim));

    builder.force_set_building(true);
    let indexed = builder.flush_sync(&index).expect("test: a skipped flush");
    builder.force_set_building(false);

    assert_eq!(indexed, 0);
    assert_eq!(builder.pending_len(), 5, "the buffer is untouched");
    assert_eq!(index.len(), 0, "the index did not change");
}

#[test]
fn test_merge_threshold_accessor() {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 5000,
        ..default_config()
    };
    let builder = AsyncIndexBuilder::new(config);
    assert_eq!(builder.merge_threshold(), 5000);
}

#[test]
fn test_config_serde_roundtrip() {
    let config = AsyncIndexBuilderConfig {
        merge_threshold: 5000,
        segment_count: Some(8),
    };
    let json = serde_json::to_string(&config).expect("serialize");
    let restored: AsyncIndexBuilderConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.merge_threshold, 5000);
    assert_eq!(restored.segment_count, Some(8));
}

#[test]
fn test_config_serde_defaults() {
    let json = "{}";
    let config: AsyncIndexBuilderConfig = serde_json::from_str(json).expect("deserialize empty");
    assert_eq!(config.merge_threshold, 10_000);
    assert!(config.segment_count.is_none());
}

/// Legacy `config.json` files persisted before the removal of
/// `sync_mode` must continue to deserialize. serde ignores unknown
/// fields by default, so the stale value is silently dropped.
#[test]
fn test_config_legacy_sync_mode_field_is_ignored() {
    let legacy_json = r#"{"merge_threshold": 500, "segment_count": 4, "sync_mode": true}"#;
    let config: AsyncIndexBuilderConfig =
        serde_json::from_str(legacy_json).expect("legacy config must still deserialize");
    assert_eq!(config.merge_threshold, 500);
    assert_eq!(config.segment_count, Some(4));
}

/// A slot no arena ever held: `link_placed` reports it as unlinked, then
/// fails to read its vector back — the error its `# Errors` section
/// documents, on the path this builder drains.
const ABSENT_SLOT: usize = 4_242;

/// A failed link must not consume the queue: `link_placed` links nothing
/// when it fails, and an id dropped from the queue then would stay out of
/// every search until crash recovery or a vacuum found its slot.
#[test]
fn a_failed_link_leaves_the_ids_on_the_queue() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    index
        .mappings
        .assign(7, crate::index::hnsw::Placed::for_test(ABSENT_SLOT));
    builder.enqueue_placed([7]);
    assert_eq!(builder.pending_len(), 1, "premise: one id is queued");

    let err = builder
        .flush_sync(&index)
        .expect_err("a slot outside the arena fails the link");

    assert!(
        matches!(err, crate::error::Error::Internal(_)),
        "the arena reported the missing slot: {err}"
    );
    assert_eq!(
        builder.pending_len(),
        1,
        "the id is still queued, so a later flush still links it"
    );
    builder
        .flush_sync(&index)
        .expect_err("the flush flag was cleared: the next flush runs, and fails the same way");
}
