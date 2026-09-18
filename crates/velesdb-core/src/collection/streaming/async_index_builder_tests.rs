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
    assert_eq!(builder.buffer_len(), 0);
    assert!(!builder.is_building());
}

#[test]
fn test_enqueue_adds_vectors() {
    let builder = AsyncIndexBuilder::new(default_config());
    let vectors = vec![(1_u64, vec![1.0_f32, 0.0, 0.0])];
    let triggered = builder.enqueue(vectors);
    assert!(!triggered);
    assert_eq!(builder.buffer_len(), 1);
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
fn test_drain_buffer_returns_all_and_empties() {
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue(vec![(1, vec![1.0, 0.0]), (2, vec![0.0, 1.0])]);
    assert_eq!(builder.buffer_len(), 2);

    let drained = builder.drain_buffer();
    assert_eq!(drained.len(), 2);
    assert_eq!(builder.buffer_len(), 0);
}

#[test]
fn test_search_buffer_finds_vectors() {
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue(vec![
        (1, vec![1.0, 0.0, 0.0]),
        (2, vec![0.0, 1.0, 0.0]),
        (3, vec![0.0, 0.0, 1.0]),
    ]);

    let results = builder.search_buffer(&[1.0, 0.0, 0.0], 2, DistanceMetric::Cosine);
    assert!(!results.is_empty());
    // For cosine, the identical vector should be first
    assert_eq!(results[0].0, 1);
}

#[test]
fn test_search_buffer_empty() {
    let builder = AsyncIndexBuilder::new(default_config());
    let results = builder.search_buffer(&[1.0, 0.0], 5, DistanceMetric::Cosine);
    assert!(results.is_empty());
}

#[test]
fn test_flush_sync_indexes_vectors() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());

    // Enqueue some vectors
    let vectors = basis_vectors(20, dim);

    builder.enqueue(vectors);
    assert_eq!(builder.buffer_len(), 20);

    // Flush synchronously
    let indexed = builder.flush_sync(&index).unwrap();
    assert_eq!(indexed, 20);
    assert_eq!(builder.buffer_len(), 0);
    assert_eq!(index.len(), 20);
}

#[test]
fn test_flush_sync_empty_buffer() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    let indexed = builder.flush_sync(&index).unwrap();
    assert_eq!(indexed, 0);
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

#[test]
fn test_trigger_build_async_indexes_in_background() {
    let dim = 4;
    let index = std::sync::Arc::new(make_index(dim));
    let builder = AsyncIndexBuilder::new(default_config());

    // Enqueue vectors
    let vectors = basis_vectors(20, dim);

    builder.enqueue(vectors);
    assert_eq!(builder.buffer_len(), 20);

    // Trigger async build
    builder.trigger_build_async(&index);

    // Wait for background thread to finish (poll with timeout)
    let start = std::time::Instant::now();
    while builder.is_building() {
        std::thread::sleep(std::time::Duration::from_millis(10));
        if start.elapsed() > std::time::Duration::from_secs(10) {
            panic!("background build did not complete within 10s");
        }
    }

    // Buffer should be drained and index populated
    assert_eq!(builder.buffer_len(), 0);
    assert_eq!(index.len(), 20);
}

#[test]
fn test_trigger_build_async_noop_when_empty() {
    let dim = 4;
    let index = std::sync::Arc::new(make_index(dim));
    let builder = AsyncIndexBuilder::new(default_config());

    // Trigger on empty buffer — should be a no-op
    builder.trigger_build_async(&index);

    // Should not be building (empty buffer returns immediately)
    assert!(!builder.is_building());
    assert_eq!(index.len(), 0);
}

/// Verifies that `trigger_build_async` is a no-op when a build is already in
/// progress.
///
/// The previous implementation polled `is_building()` with a sleep between the
/// first `trigger_build_async` call and the second, which was racy: on fast
/// hardware the background thread finished and cleared `building` before the
/// second trigger was called, causing both triggers to run and the assertion
/// `buffer_len() == 5` to fail with 0.
///
/// Fix: inject the "already building" state directly via
/// `force_set_building(true)` without spawning a real background thread,
/// making the test fully deterministic.
#[test]
fn test_trigger_build_async_skips_when_already_building() {
    let dim = 4;
    let index = std::sync::Arc::new(make_index(dim));
    let builder = AsyncIndexBuilder::new(default_config());

    // Simulate a build already in progress — no real thread needed.
    builder.force_set_building(true);
    assert!(builder.is_building());

    // Enqueue vectors that should stay buffered because the trigger is skipped.
    let vectors = basis_vectors(5, dim);
    builder.enqueue(vectors);
    assert_eq!(builder.buffer_len(), 5);

    // Call trigger — must be a no-op because `building` is already true.
    builder.trigger_build_async(&index);

    // Buffer untouched; no thread was spawned; index is empty.
    assert_eq!(
        builder.buffer_len(),
        5,
        "buffer must not be drained when already building"
    );
    assert_eq!(
        index.len(),
        0,
        "index must not change when trigger is skipped"
    );
    assert!(builder.is_building(), "building flag must still be set");

    // Restore invariant so builder is not left in a permanently-locked state.
    builder.force_set_building(false);
}

/// A slot no arena ever held: `link_placed` reports it as unlinked, then
/// fails to read its vector back — the error its `# Errors` section
/// documents, on the path this builder drains.
const ABSENT_SLOT: usize = 4_242;

/// A failed link must not consume the queue. `flush_sync` empties `placed`
/// before it links, and `link_placed` links nothing when it fails: without
/// the requeue the ids are gone and no retry can reach their slots, which
/// stay out of every search until crash recovery or a vacuum finds them.
#[test]
fn a_failed_link_puts_the_drained_ids_back_on_the_queue() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    index
        .mappings
        .assign(7, crate::index::hnsw::Placed::for_test(ABSENT_SLOT));
    builder.enqueue_placed([7]);
    assert_eq!(builder.placed_len(), 1, "premise: one id is queued");

    let err = builder
        .flush_sync(&index)
        .expect_err("a slot outside the arena fails the link");

    assert!(
        matches!(err, crate::error::Error::Internal(_)),
        "the arena reported the missing slot: {err}"
    );
    assert_eq!(
        builder.placed_len(),
        1,
        "the drained id is back on the queue, so a later flush still links it"
    );
    assert!(!builder.is_building(), "the build flag is cleared");
}

/// `trigger_build_async` drains the vector buffer only, so it must refuse
/// while placed ids wait: a background build that reported done with slots
/// still unlinked is the hazard #488 Task 4 would wire.
#[test]
fn trigger_build_async_refuses_while_placed_ids_wait() {
    let dim = 4;
    let index = std::sync::Arc::new(make_index(dim));
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue_placed([7]);
    let vectors = basis_vectors(5, dim);
    builder.enqueue(vectors);

    builder.trigger_build_async(&index);

    assert_eq!(
        builder.buffer_len(),
        5,
        "the vector buffer is not drained past the placed queue"
    );
    assert_eq!(builder.placed_len(), 1, "the placed queue is untouched");
    assert!(!builder.is_building(), "no background build was started");
    assert_eq!(index.len(), 0, "the index did not change");
}
