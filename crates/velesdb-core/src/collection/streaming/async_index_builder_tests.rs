//! Tests for `AsyncIndexBuilder`.

use super::async_index_builder::{AsyncIndexBuilder, AsyncIndexBuilderConfig, FlushOutcome};
use crate::distance::DistanceMetric;
use crate::index::hnsw::HnswIndex;
use crate::index::VectorIndex;

fn default_config() -> AsyncIndexBuilderConfig {
    AsyncIndexBuilderConfig {
        merge_threshold: 100,
        segment_count: Some(2),
    }
}

fn make_index(dim: usize) -> HnswIndex {
    HnswIndex::new(dim, DistanceMetric::Cosine).expect("test index creation")
}

/// `count` stored vectors, id `i` carrying the `i % dim`-th basis vector.
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

/// The ids of `points`, as `enqueue` takes them.
fn ids_of(points: &[(u64, Vec<f32>)]) -> Vec<u64> {
    points.iter().map(|(id, _)| *id).collect()
}

/// A stand-in for `Collection::index_current` over storage holding
/// `points`: inserts what `points` holds for each id asked about and
/// settles it, and settles an id it does not hold, as a deleted one.
fn indexing<'a>(
    index: &'a HnswIndex,
    points: &'a [(u64, Vec<f32>)],
) -> impl Fn(&[u64]) -> Vec<u64> + 'a {
    move |ids| {
        let present: Vec<(u64, Vec<f32>)> = ids
            .iter()
            .filter_map(|id| points.iter().find(|(stored, _)| stored == id).cloned())
            .collect();
        let mut settled: Vec<u64> = ids
            .iter()
            .copied()
            .filter(|id| !present.iter().any(|(stored, _)| stored == id))
            .collect();
        settled.extend(index.insert_batch_unless_indexed(&present).settled);
        settled
    }
}

/// Storage that holds nothing: every id is settled as deleted.
fn nothing_stored(ids: &[u64]) -> Vec<u64> {
    ids.to_vec()
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
    let triggered = builder.enqueue([1_u64]);
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

    assert!(!builder.enqueue([1]));
    assert!(!builder.enqueue([2]));
    assert!(builder.enqueue([3]));
}

/// Each pending id once, sorted: an id queued twice, and an id both placed
/// and buffered, are one pending point.
#[test]
fn pending_ids_names_each_waiting_id_once() {
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue_placed([7, 3, 7]);
    builder.enqueue_placed([3]);
    builder.enqueue([9, 3, 9]);

    assert_eq!(builder.pending_ids(), vec![3, 7, 9]);
}

#[test]
fn test_flush_sync_indexes_vectors() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());

    let points = basis_vectors(20, dim);
    builder.enqueue(ids_of(&points));
    assert_eq!(builder.pending_len(), 20);

    let outcome = builder
        .flush_sync(&index, &indexing(&index, &points))
        .unwrap();
    assert_eq!(outcome.indexed, 20);
    assert_eq!(outcome.kept, 0);
    assert_eq!(builder.pending_len(), 0);
    assert_eq!(index.len(), 20);
}

#[test]
fn test_flush_sync_empty_buffer() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    let outcome = builder.flush_sync(&index, &nothing_stored).unwrap();
    assert_eq!(outcome, FlushOutcome::default());
}

/// A flush indexes what storage holds for each queued id when it runs: a
/// point overwritten since it was queued is indexed on its new vector, a
/// point deleted since is not indexed, and both leave the queue.
#[test]
fn a_flush_inserts_what_storage_holds_now() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());
    let queued = basis_vectors(2, dim);
    builder.enqueue(ids_of(&queued));
    let now = vec![(0_u64, vec![0.0_f32, 0.0, 1.0, 0.0])];

    let settled = builder
        .flush_sync(&index, &indexing(&index, &now))
        .unwrap()
        .indexed;

    assert_eq!(settled, 2, "one id inserted, one found deleted");
    assert_eq!(index.len(), 1);
    assert!(builder.pending_ids().is_empty(), "both ids left the queue");
    let hits = index.search(&now[0].1, 1);
    assert_eq!(hits[0].id, 0);
    assert!(
        (hits[0].score - 1.0).abs() < 1e-5,
        "id 0 is indexed on its new vector: {}",
        hits[0].score
    );
}

/// A flush that finds another in progress indexes nothing and leaves both
/// queues — and so what `pending_ids` names — as they were.
#[test]
fn flush_sync_skips_while_another_flush_runs() {
    let dim = 4;
    let index = make_index(dim);
    let builder = AsyncIndexBuilder::new(default_config());
    let points = basis_vectors(5, dim);
    builder.enqueue(ids_of(&points));

    builder.force_set_building(true);
    let outcome = builder
        .flush_sync(&index, &indexing(&index, &points))
        .expect("test: a skipped flush");
    builder.force_set_building(false);

    assert_eq!(outcome, FlushOutcome::default());
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
        .flush_sync(&index, &nothing_stored)
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
        .flush_sync(&index, &nothing_stored)
        .expect_err("the flush flag was cleared: the next flush runs, and fails the same way");
}

/// An id the flush's indexing did not settle (its read or insert failed)
/// stays queued, and searchable, for the next flush. The flush succeeds and
/// counts it: a state a later flush retries is no failure for its caller.
/// An id queued twice is counted once.
#[test]
fn an_id_a_flush_did_not_settle_stays_queued() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue([1, 2, 3, 1]);

    let outcome = builder
        .flush_sync(&index, &|_| vec![2])
        .expect("test: kept ids do not fail the flush");

    assert_eq!(outcome.kept, 2, "ids 1 and 3, each once");
    assert_eq!(outcome.indexed, 1);
    assert_eq!(builder.pending_ids(), vec![1, 3]);
}

/// An id queued twice is handed to the indexing once.
#[test]
fn an_id_queued_twice_is_indexed_once() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue([5, 5, 6]);
    let asked = std::cell::RefCell::new(Vec::new());

    builder
        .flush_sync(&index, &|ids| {
            asked.borrow_mut().extend_from_slice(ids);
            ids.to_vec()
        })
        .expect("test: flush");

    let mut asked = asked.into_inner();
    asked.sort_unstable();
    assert_eq!(asked, vec![5, 6]);
    assert!(builder.pending_ids().is_empty());
}

/// A flush that panics half-way does not leave the builder's flag set:
/// the next flush runs.
#[test]
fn a_flush_that_panics_lets_the_next_one_run() {
    let index = make_index(4);
    let builder = AsyncIndexBuilder::new(default_config());
    builder.enqueue([1]);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = builder.flush_sync(&index, &|_| panic!("test: indexing fails half-way"));
    }));
    assert!(unwound.is_err(), "premise: the flush panicked");

    let settled = builder
        .flush_sync(&index, &nothing_stored)
        .expect("test: flush")
        .indexed;

    assert_eq!(settled, 1, "the second flush ran");
    assert!(builder.pending_ids().is_empty());
}
