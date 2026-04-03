//! Tests for `AsyncIndexBuilder`.

use super::async_index_builder::{AsyncIndexBuilder, AsyncIndexBuilderConfig};
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswIndex;

fn default_config() -> AsyncIndexBuilderConfig {
    AsyncIndexBuilderConfig {
        merge_threshold: 100,
        segment_count: Some(2),
        sync_mode: false,
    }
}

fn make_index(dim: usize) -> HnswIndex {
    HnswIndex::new(dim, DistanceMetric::Cosine).expect("test index creation")
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
        sync_mode: false,
    };
    let builder = AsyncIndexBuilder::new(config);

    assert!(!builder.enqueue(vec![(1, vec![1.0])]));
    assert!(!builder.enqueue(vec![(2, vec![2.0])]));
    assert!(builder.enqueue(vec![(3, vec![3.0])]));
}

#[test]
fn test_drain_buffer_returns_all_and_empties() {
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue(vec![
        (1, vec![1.0, 0.0]),
        (2, vec![0.0, 1.0]),
    ]);
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
    let vectors: Vec<(u64, Vec<f32>)> = (0..20)
        .map(|i| {
            let mut v = vec![0.0_f32; dim];
            v[i % dim] = 1.0;
            (i as u64, v)
        })
        .collect();

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
        sync_mode: true,
    };
    let json = serde_json::to_string(&config).expect("serialize");
    let restored: AsyncIndexBuilderConfig =
        serde_json::from_str(&json).expect("deserialize");
    assert_eq!(restored.merge_threshold, 5000);
    assert_eq!(restored.segment_count, Some(8));
    assert!(restored.sync_mode);
}

#[test]
fn test_config_serde_defaults() {
    let json = "{}";
    let config: AsyncIndexBuilderConfig =
        serde_json::from_str(json).expect("deserialize empty");
    assert_eq!(config.merge_threshold, 10_000);
    assert!(config.segment_count.is_none());
    assert!(!config.sync_mode);
}
