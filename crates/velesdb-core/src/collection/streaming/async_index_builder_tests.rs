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
    let index = make_index(2);
    let results = builder.search_pending(&index, &[1.0, 0.0], 5, DistanceMetric::Cosine);
    assert!(results.is_empty());
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

#[test]
fn search_pending_finds_buffered_vectors() {
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue(vec![
        (1, vec![1.0, 0.0, 0.0]),
        (2, vec![0.0, 1.0, 0.0]),
        (3, vec![0.0, 0.0, 1.0]),
    ]);
    let index = make_index(3);

    let results = builder.search_pending(&index, &[1.0, 0.0, 0.0], 2, DistanceMetric::Cosine);
    assert_eq!(results.len(), 2, "truncated to k");
    // For cosine, the identical vector should be first
    assert_eq!(results[0].0, 1);
}

/// Places `ids`' vectors in `index`'s arena and maps them, as `upsert_bulk`'s
/// direct writer does, without linking them.
fn place(index: &HnswIndex, vectors: &[(u64, Vec<f32>)]) {
    let refs: Vec<(u64, &[f32])> = vectors.iter().map(|(id, v)| (*id, v.as_slice())).collect();
    crate::index::hnsw::direct_writer::DirectVectorWriter::new(index)
        .write_batch_direct(&refs)
        .expect("test: direct write")
        .expect("test: an index with exact-distance features on places");
}

/// A placed id is scored from the slot it is mapped to, merged with the
/// buffer, and a deleted one is skipped: the graph cannot reach either yet.
#[test]
fn search_pending_scores_placed_ids_from_the_arena() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());
    place(&index, &basis_vectors(4, dim));
    builder.enqueue_placed([0, 1, 2, 3]);
    builder.enqueue(vec![(9, vec![0.9, 0.1, 0.0, 0.0])]);
    index.remove(1);

    let results = builder.search_pending(&index, &[1.0, 0.0, 0.0, 0.0], 3, DistanceMetric::Cosine);
    let ids: Vec<u64> = results.iter().map(|&(id, _)| id).collect();
    assert_eq!(
        ids[..2],
        [0, 9],
        "the placed and the buffered match, best first"
    );
    assert!(!ids.contains(&1), "a deleted id is not scored: {ids:?}");
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
/// queues — and so what `search_pending` finds — as they were.
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
