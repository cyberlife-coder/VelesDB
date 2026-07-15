#![cfg(all(test, feature = "persistence"))]
//! Tests for the open-time HNSW reload + 3-pass reconciliation (ENG-01).
//!
//! `Collection::open` loads the persisted HNSW index (gated on
//! `native_meta.bin`) and reconciles it against the WAL-replayed vector
//! storage: gap recovery (pass 1), orphan removal (pass 2), and stale
//! WAL-touched re-upserts (pass 3). Load failures fall back to the
//! pre-existing full rebuild.

#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use crate::collection::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use std::path::PathBuf;

/// Creates N distinct 4-dim points with IDs starting from `start`.
fn make_points(start: u64, n: u64) -> Vec<Point> {
    (start..start + n)
        .map(|i| {
            let f = i as f32;
            Point::without_payload(i, vec![f, f + 1.0, f + 2.0, f + 3.0])
        })
        .collect()
}

/// Brief test 1 — true round-trip proving the LOAD (not the rebuild):
/// a vector upserted AFTER `flush_full` must win over the persisted (stale)
/// index entry on reopen (gate + pass 3).
#[test]
fn test_reopen_reconciles_stale_wal_upsert_into_loaded_index() {
    let temp = tempfile::tempdir().expect("temp dir");
    let vec_a = vec![1.0, 0.0, 0.0, 0.0];
    let vec_b = vec![0.0, 1.0, 0.0, 0.0];

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");
        coll.upsert(vec![Point::without_payload(1, vec_a)])
            .expect("upsert A");
        coll.flush_full().expect("flush_full");
        // Stale window: the persisted index still holds A.
        coll.upsert(vec![Point::without_payload(1, vec_b.clone())])
            .expect("upsert B");
        // Drop WITHOUT flush_full: only the vector WAL knows about B.
    }
    assert!(
        temp.path().join("native_meta.bin").exists(),
        "flush_full must have persisted the index"
    );

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    // The loaded index assigned internal idx 0 to A; the pass-3 re-upsert of
    // B consumes idx 1. A rebuilt-from-scratch index would stop at next_idx
    // 1, so this proves the persisted graph was LOADED, not rebuilt.
    assert_eq!(
        reopened.storage.index.mappings.next_idx(),
        2,
        "stale re-upsert must run on the LOADED index (not a rebuild)"
    );
    let results = reopened.search(&vec_b, 1).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].point.id, 1);
    assert!(
        results[0].score > 0.99,
        "id 1 must match vector B (~distance 0), got score {}",
        results[0].score
    );
}

/// Brief test 2 — delete-orphan: an id deleted after `flush_full` must be
/// purged from the loaded index on reopen (pass 2).
#[test]
fn test_reopen_removes_deleted_id_from_loaded_index() {
    let temp = tempfile::tempdir().expect("temp dir");

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");
        coll.upsert(make_points(0, 10)).expect("upsert");
        coll.flush_full().expect("flush_full");
        coll.delete(&[3]).expect("delete");
        // Drop WITHOUT flush_full: the persisted index still maps id 3.
    }

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(reopened.len(), 9, "deleted point must stay deleted");
    assert!(
        reopened.get(&[3])[0].is_none(),
        "get(3) must return None after reopen"
    );
    let results = reopened.search(&[3.0, 4.0, 5.0, 6.0], 10).expect("search");
    assert!(
        results.iter().all(|r| r.point.id != 3),
        "search must never return the deleted id"
    );
}

/// Brief test 3 — corrupted generation marker: a divergent
/// `native_hnsw.gen` must NOT fail the open; the load is rejected and the
/// index is rebuilt from storage.
#[test]
fn test_reopen_rebuilds_on_generation_mismatch() {
    let temp = tempfile::tempdir().expect("temp dir");

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");
        coll.upsert(make_points(0, 10)).expect("upsert");
        coll.flush_full().expect("flush_full");
    }

    // Stamp a generation that disagrees with the meta commit point.
    let bogus = postcard::to_allocvec(&999_u64).expect("encode");
    std::fs::write(temp.path().join("native_hnsw.gen"), bogus).expect("write gen");

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen must succeed");
    assert_eq!(reopened.len(), 10);
    let results = reopened.search(&[0.0, 1.0, 2.0, 3.0], 10).expect("search");
    assert_eq!(results.len(), 10, "all points searchable after rebuild");
}

/// Brief test 4 — foreign meta: a `native_meta.bin` whose dimension does
/// not match the collection config must trigger the rebuild fallback, not
/// an open failure.
#[test]
fn test_reopen_rebuilds_on_foreign_meta_dimension() {
    use crate::index::hnsw::persistence::{load_meta, save_meta, HnswMeta};

    let temp = tempfile::tempdir().expect("temp dir");

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");
        coll.upsert(make_points(0, 5)).expect("upsert");
        coll.flush_full().expect("flush_full");
    }

    // Rewrite the meta with a foreign dimension, keeping the generation so
    // the sidecar consistency check is not what rejects it.
    let meta = load_meta(temp.path()).expect("load meta");
    save_meta(
        temp.path(),
        &HnswMeta {
            dimension: 8,
            ..meta
        },
    )
    .expect("save foreign meta");

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen must succeed");
    assert_eq!(reopened.len(), 5);
    let results = reopened.search(&[0.0, 1.0, 2.0, 3.0], 5).expect("search");
    assert_eq!(results.len(), 5, "all points searchable after rebuild");
}

/// Brief test 5 — gap after a partial flush: points inserted after
/// `flush_full` and only `flush()`-ed must be recovered on reopen
/// (loaded index + pass 1 gap recovery).
#[test]
fn test_reopen_recovers_gap_on_top_of_loaded_index() {
    let temp = tempfile::tempdir().expect("temp dir");

    {
        let coll = Collection::create(PathBuf::from(temp.path()), 4, DistanceMetric::Cosine)
            .expect("create");
        coll.upsert(make_points(0, 5)).expect("upsert 1");
        coll.flush_full().expect("flush_full");
        coll.upsert(make_points(5, 5)).expect("upsert 2");
        coll.flush().expect("flush"); // fast path: no HNSW save
    }

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(reopened.len(), 10);
    let results = reopened.search(&[5.0, 6.0, 7.0, 8.0], 10).expect("search");
    assert_eq!(results.len(), 10, "all 10 points searchable after reopen");
}

/// Brief test 6 — alpha round-trip at the index level: a custom VAMANA
/// alpha is persisted in the v2 `.graph` header and restored by
/// `HnswIndex::load` (no `config.json` involved, so this isolates the
/// header persistence from the params-based rebuild fallback).
#[test]
fn test_hnsw_index_alpha_round_trip_v2_header() {
    use crate::index::HnswIndex;

    let temp = tempfile::tempdir().expect("temp dir");
    let params = crate::index::hnsw::HnswParams::auto(4).with_alpha(1.7);
    let index = HnswIndex::with_params(4, DistanceMetric::Cosine, params).expect("build");
    let v = [1.0_f32, 0.0, 0.0, 0.0];
    assert_eq!(index.insert_batch_parallel(vec![(1_u64, &v[..])]), 1);
    index.save(temp.path()).expect("save");

    let loaded = HnswIndex::load(temp.path(), 4, DistanceMetric::Cosine).expect("load");
    assert_eq!(
        loaded.inner.read().alpha(),
        1.7,
        "custom alpha must survive the .graph v2 header round-trip"
    );
}

/// Brief test 6 (end-to-end) — alpha round-trip: a custom VAMANA alpha must
/// survive `flush_full` + reopen at the collection level.
#[test]
fn test_alpha_round_trip_through_reopen() {
    let temp = tempfile::tempdir().expect("temp dir");
    let params = crate::index::hnsw::HnswParams::auto(4).with_alpha(1.5);

    {
        let coll = Collection::create_with_hnsw_params(
            PathBuf::from(temp.path()),
            4,
            DistanceMetric::Cosine,
            crate::quantization::StorageMode::Full,
            params,
        )
        .expect("create");
        assert_eq!(coll.storage.index.inner.read().alpha(), 1.5);
        coll.upsert(make_points(0, 3)).expect("upsert");
        coll.flush_full().expect("flush_full");
    }

    let reopened = Collection::open(PathBuf::from(temp.path())).expect("reopen");
    assert_eq!(
        reopened.storage.index.inner.read().alpha(),
        1.5,
        "custom alpha must survive the save/load round-trip"
    );
}
