//! `apply_advanced_config` takes effect, or says it will only at reopen.
//!
//! The deferred indexer and the async index builder used to be built only
//! when a collection was assembled (create or open), so setting either on an
//! open collection — `POST /collections` does it right after creating one —
//! rewrote `config.json` and changed nothing until the next open, silently.

use crate::collection::streaming::{AsyncIndexBuilderConfig, DeferredIndexerConfig, PendingReopen};
use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use crate::quantization::StorageMode;

const DIM: usize = 4;

fn plain_collection(path: std::path::PathBuf) -> Collection {
    Collection::create_with_options(path, DIM, DistanceMetric::Cosine, StorageMode::Full)
        .expect("test: create a collection without streaming settings")
}

fn deferred(merge_threshold: usize) -> DeferredIndexerConfig {
    DeferredIndexerConfig {
        enabled: true,
        merge_threshold,
        ..DeferredIndexerConfig::default()
    }
}

fn three_points() -> Vec<Point> {
    (1..=3u8)
        .map(|i| {
            let x = f32::from(i);
            Point::without_payload(u64::from(i), vec![x, 1.0, 0.5, -x])
        })
        .collect()
}

fn builder(merge_threshold: usize) -> AsyncIndexBuilderConfig {
    AsyncIndexBuilderConfig {
        merge_threshold,
        segment_count: None,
    }
}

#[test]
fn enabling_deferred_indexing_on_an_open_collection_takes_effect_now() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = plain_collection(dir.path().join("c"));

    coll.apply_advanced_config(None, Some(Some(deferred(64))), None)
        .expect("test: apply");

    let live = coll
        .deferred_indexer()
        .expect("the indexer is installed live");
    assert_eq!(live.config(), &deferred(64));
    assert!(coll.pending_reopen().is_empty());

    coll.upsert_bulk(&three_points())
        .expect("test: upsert_bulk");
    assert_eq!(
        live.pending_count(),
        3,
        "the next write is buffered by the indexer just installed"
    );
}

#[test]
fn enabling_the_async_builder_on_an_open_collection_takes_effect_now() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = plain_collection(dir.path().join("c"));

    coll.apply_advanced_config(None, None, Some(Some(builder(500))))
        .expect("test: apply");

    let live = coll
        .async_index_builder()
        .expect("the builder is installed live");
    assert_eq!(live.config(), &builder(500));
    assert!(coll.pending_reopen().is_empty());

    coll.upsert_bulk(&three_points())
        .expect("test: upsert_bulk");
    assert_eq!(
        live.pending_len(),
        3,
        "the next bulk write takes the V2 path through the builder just installed"
    );
}

#[test]
fn the_live_install_is_shared_by_every_handle_on_the_collection() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = plain_collection(dir.path().join("c"));
    // The registry and each caller hold their own clone of the collection.
    let other_handle = coll.clone();

    coll.apply_advanced_config(None, None, Some(Some(builder(500))))
        .expect("test: apply");

    assert!(other_handle.async_index_builder().is_some());
}

#[test]
fn changing_a_live_builder_is_reported_and_applies_at_reopen() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let path = dir.path().join("c");
    let coll = Collection::create_with_async_builder(
        path.clone(),
        DIM,
        DistanceMetric::Cosine,
        builder(500),
    )
    .expect("test: create with a builder");

    coll.apply_advanced_config(None, None, Some(Some(builder(9_000))))
        .expect("test: apply");

    assert_eq!(
        coll.pending_reopen(),
        vec![PendingReopen::AsyncIndexBuilder]
    );
    let live = coll
        .async_index_builder()
        .expect("the old builder stays live");
    assert_eq!(
        live.config(),
        &builder(500),
        "a live builder is never swapped"
    );
    drop(live);
    drop(coll);

    let reopened = Collection::open(path).expect("test: reopen");
    assert!(reopened.pending_reopen().is_empty());
    let live = reopened.async_index_builder().expect("built at open");
    assert_eq!(live.config(), &builder(9_000));
}

#[test]
fn disabling_live_deferred_indexing_is_reported() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = plain_collection(dir.path().join("c"));
    coll.apply_advanced_config(None, Some(Some(deferred(64))), None)
        .expect("test: enable");

    coll.apply_advanced_config(None, Some(None), None)
        .expect("test: disable");

    assert_eq!(coll.pending_reopen(), vec![PendingReopen::DeferredIndexing]);
    assert!(coll.deferred_indexer().is_some(), "still live until reopen");
}

#[test]
fn a_disabled_deferred_config_is_not_pending() {
    let dir = tempfile::tempdir().expect("test: temp dir");
    let coll = plain_collection(dir.path().join("c"));
    let off = DeferredIndexerConfig {
        enabled: false,
        ..DeferredIndexerConfig::default()
    };

    coll.apply_advanced_config(None, Some(Some(off)), None)
        .expect("test: apply");

    assert!(coll.deferred_indexer().is_none());
    assert!(
        coll.pending_reopen().is_empty(),
        "a disabled indexer is what an open would build: nothing"
    );
}

#[test]
fn pending_reopen_names_the_config_field() {
    assert_eq!(
        PendingReopen::DeferredIndexing.field_name(),
        "deferred_indexing"
    );
    assert_eq!(
        PendingReopen::AsyncIndexBuilder.field_name(),
        "async_index_builder"
    );
}
