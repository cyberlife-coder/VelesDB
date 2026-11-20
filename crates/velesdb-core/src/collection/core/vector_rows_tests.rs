//! A vector collection's rows are its vectors (`RowUniverse::Vectors`): no
//! row write stores a payload for an id without one — `upsert_metadata`
//! checks it under the id's node lock
//! (`Collection::write_payloads_where_vectors`), and an upsert, bulk or not,
//! stores both under those locks (`Collection::write_rows`), against a
//! delete racing them — and no read reaches what an earlier version stored there — a
//! payload `upsert_metadata` wrote without a vector, with its BM25 document
//! and sparse vectors — which stays on disk, unreachable, until a delete of
//! its id removes it. The one write that stores such a payload on purpose is
//! `store_node_payload`, the payload of a graph edge's endpoint, which is no
//! row either (`row_universe.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::size::StoreEffect;
use crate::collection::search::query::row_universe::RowUniverse;
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::index::sparse::SparseVector;
use crate::point::Point;
use crate::storage::PayloadStorage;

fn vectors(dir: &Path) -> Collection {
    Collection::create(PathBuf::from(dir), 2, DistanceMetric::Cosine).expect("test: collection")
}

fn ids(results: &[crate::point::SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

fn sparse(weight: f32) -> SparseVector {
    SparseVector::new(vec![(1, weight), (2, weight)])
}

/// What an earlier version left for `id` when `upsert_metadata` stored a
/// payload there without a vector: the payload, its BM25 document and its
/// histogram counts — and sparse vectors, which a bulk write could add.
fn plant_legacy_row(collection: &Collection, id: u64, payload: &Value) {
    collection
        .write_payload_store(
            RowUniverse::Vectors,
            &[id],
            StoreEffect::Stores,
            |storage| storage.store(id, payload),
        )
        .expect("test: legacy payload");
    collection
        .apply_payload_effects(&[(id, Some(payload))], &[None])
        .expect("test: legacy BM25 document");
    collection
        .apply_sparse_batch_upsert(&[Point::sparse_only(id, sparse(10.0), None)])
        .expect("test: legacy sparse vector");
}

/// One row (1) whose text and sparse vector match weakly, and two legacy
/// leftovers (10, 11) whose text and sparse vectors match better.
fn collection_with_leftovers(dir: &Path) -> Collection {
    let collection = vectors(dir);
    let words = json!({"content": "words words words"});
    plant_legacy_row(&collection, 10, &words);
    plant_legacy_row(&collection, 11, &words);
    collection
        .upsert([Point {
            id: 1,
            vector: vec![1.0, 0.0],
            payload: Some(json!({"content": "words alpha"})),
            sparse_vectors: Some(std::collections::BTreeMap::from([(
                String::new(),
                sparse(1.0),
            )])),
        }])
        .expect("test: row");
    collection
}

fn sparse_params() -> HashMap<String, Value> {
    HashMap::from([("s".to_string(), json!({"1": 1.0, "2": 1.0}))])
}

#[test]
fn upsert_metadata_on_a_vector_collection_refuses_an_id_without_a_vector() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    collection
        .upsert([Point::new(1, vec![1.0, 0.0], Some(json!({"v": 1})))])
        .expect("test: row");

    let refused = collection.upsert_metadata([
        Point::metadata_only(1, json!({"v": 2})),
        Point::metadata_only(10, json!({"content": "orphan"})),
    ]);

    assert_eq!(refused.map_err(|e| e.code()), Err("VELES-016"));
    assert_eq!(
        collection.get(&[1])[0]
            .as_ref()
            .and_then(|p| p.payload.clone()),
        Some(json!({"v": 1})),
        "a refused batch writes nothing, not even its valid ids"
    );
    assert_eq!(stored_ids(&collection), vec![1], "no payload stored for 10");
    assert!(collection
        .text_search("orphan", 10)
        .expect("test")
        .is_empty());
}

#[test]
fn upsert_metadata_on_a_vector_collection_replaces_the_payload_of_a_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    collection
        .upsert([Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"content": "old"})),
        )])
        .expect("test: row");

    collection
        .upsert_metadata([Point::metadata_only(1, json!({"content": "fresh"}))])
        .expect("test: a row's payload");

    let row = collection.get(&[1])[0].clone().expect("test: row");
    assert_eq!(row.vector, vec![1.0, 0.0], "the vector is kept");
    assert_eq!(row.payload, Some(json!({"content": "fresh"})));
    assert_eq!(
        ids(&collection.text_search("fresh", 10).expect("test")),
        [1]
    );
    assert!(collection.text_search("old", 10).expect("test").is_empty());
}

#[test]
fn every_enumeration_lists_the_rows_only() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection_with_leftovers(dir.path());

    assert_eq!(collection.len(), 1);
    assert_eq!(collection.all_ids(), [1]);
    assert_eq!(collection.all_point_ids(), [1]);
    let scrolled = collection.scroll_batch(None, 10, None).expect("test");
    assert_eq!(ids_of(&scrolled.points), [1], "scroll");
    assert_eq!(stored_ids(&collection), [1, 10, 11], "the leftovers stay");
}

fn ids_of(points: &[Point]) -> Vec<u64> {
    points.iter().map(|p| p.id).collect()
}

/// Every id either store holds, a row or not, ascending.
fn stored_ids(collection: &Collection) -> Vec<u64> {
    use crate::storage::VectorStorage;
    let mut ids = collection.storage.vector_storage.read().ids();
    ids.extend(collection.storage.payload_storage.read().ids());
    ids.sort_unstable();
    ids.dedup();
    ids
}

#[test]
fn a_delete_removes_the_leftovers_of_an_id_and_counts_only_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection_with_leftovers(dir.path());

    assert_eq!(collection.delete_counted(&[10]).expect("test"), 0);
    assert_eq!(stored_ids(&collection), [1, 11], "10's payload is gone");
    assert_eq!(collection.delete_counted(&[1, 11]).expect("test"), 1);
    assert!(stored_ids(&collection).is_empty());
}

#[test]
fn text_search_ranks_rows_only_before_its_top_k() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection_with_leftovers(dir.path());

    assert_eq!(ids(&collection.text_search("words", 1).expect("test")), [1]);
    let filter = crate::filter::Filter::new(crate::filter::Condition::And { conditions: vec![] });
    assert_eq!(
        ids(&collection
            .text_search_with_filter("words", 1, &filter)
            .expect("test")),
        [1]
    );
    let matched = collection
        .execute_query_str(
            "SELECT * FROM c WHERE content MATCH 'words' LIMIT 1",
            &HashMap::new(),
        )
        .expect("test: MATCH");
    assert_eq!(ids(&matched), [1], "MATCH");
    let hybrid = collection
        .hybrid_search(&[1.0, 0.0], "words", 1, None, None)
        .expect("test: hybrid");
    assert_eq!(ids(&hybrid), [1], "hybrid");
}

#[test]
fn sparse_search_ranks_rows_only_before_its_top_k() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection_with_leftovers(dir.path());

    let found = collection
        .execute_query_str(
            "SELECT * FROM c WHERE vector SPARSE_NEAR $s LIMIT 1",
            &sparse_params(),
        )
        .expect("test: SPARSE_NEAR");
    assert_eq!(ids(&found), [1], "SPARSE_NEAR");
    let view = crate::collection::VectorCollection { inner: collection };
    assert_eq!(
        ids(&view.sparse_search(&sparse(1.0), 1, "").expect("test")),
        [1],
        "VectorCollection::sparse_search"
    );
}

/// The leftovers are not purged on open: they stay on disk, unreachable.
#[test]
fn leftovers_stay_unreachable_across_a_reopen() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection_with_leftovers(dir.path());
    collection.flush_full().expect("test: flush");
    drop(collection);

    let reopened = Collection::open(PathBuf::from(dir.path())).expect("test: reopen");
    assert_eq!(stored_ids(&reopened), [1, 10, 11], "not purged");
    assert_eq!(reopened.all_ids(), [1]);
    assert_eq!(ids(&reopened.text_search("words", 1).expect("test")), [1]);
    let found = reopened
        .execute_query_str(
            "SELECT * FROM c WHERE vector SPARSE_NEAR $s LIMIT 1",
            &sparse_params(),
        )
        .expect("test: SPARSE_NEAR");
    assert_eq!(ids(&found), [1]);
}

#[test]
fn truncate_deletes_the_leftovers_and_counts_the_rows() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = crate::Database::open(dir.path()).expect("test: database");
    db.create_vector_collection("vc", 2, DistanceMetric::Cosine)
        .expect("test: collection");
    let collection = db.get_vector_collection("vc").expect("test: collection");
    let words = json!({"content": "words"});
    plant_legacy_row(&collection.inner, 10, &words);
    collection
        .upsert(vec![Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: row");

    let query = crate::velesql::Parser::parse("TRUNCATE vc").expect("test: parse");
    let result = db
        .execute_query(&query, &HashMap::new())
        .expect("test: truncate");

    assert_eq!(result[0].point.payload, Some(json!({"deleted_count": 1})));
    assert!(stored_ids(&collection.inner).is_empty());
}

/// A vector collection's every row stores a vector, so every write refuses
/// a row without one with `VELES-016` — the collection API and VelesQL
/// alike — and keeps `VELES-004` for a vector of another dimension.
#[test]
fn every_write_refuses_a_row_without_a_vector_with_veles_016() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let db = crate::Database::open(dir.path()).expect("test: database");
    db.create_vector_collection("vc", 2, DistanceMetric::Cosine)
        .expect("test: collection");
    let collection = db
        .get_vector_collection("vc")
        .expect("test: collection")
        .inner;
    let code = |written: crate::error::Result<()>| written.map_err(|e| e.code());

    assert_eq!(
        code(collection.upsert([Point::new(1, vec![], Some(json!({"a": 1})))])),
        Err("VELES-016"),
        "upsert"
    );
    assert_eq!(
        code(
            collection
                .upsert_bulk(&[Point::new(1, vec![], None)])
                .map(drop)
        ),
        Err("VELES-016"),
        "upsert_bulk"
    );
    assert_eq!(
        code(collection.upsert_metadata([Point::metadata_only(1, json!({"a": 1}))])),
        Err("VELES-016"),
        "upsert_metadata"
    );
    let params = HashMap::from([("v".to_string(), json!([]))]);
    for sql in [
        "INSERT INTO vc (id, a) VALUES (1, 1)",
        "INSERT INTO vc (id, vector) VALUES (1, $v)",
    ] {
        let query = crate::velesql::Parser::parse(sql).expect("test: parse");
        assert_eq!(
            code(db.execute_query(&query, &params).map(drop)),
            Err("VELES-016"),
            "{sql}"
        );
    }
    assert_eq!(
        code(collection.upsert([Point::new(1, vec![1.0], None)])),
        Err("VELES-004"),
        "a vector of another dimension"
    );
    assert!(stored_ids(&collection).is_empty(), "nothing was written");
}

/// Config ranks first in the lock order, so a write learns which kind of
/// collection it writes ([`Collection::row_universe`]) before it takes a
/// storage guard — which `row_universe` checks under `cfg(test)`. A vector
/// collection's payload write used to read it under `vector_storage`'s read
/// guard.
#[test]
fn a_payload_write_reads_the_configuration_before_any_storage_guard() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    collection
        .upsert([Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: row");
    collection
        .upsert_metadata([Point::metadata_only(1, json!({"v": 1}))])
        .expect("test: the row's payload");
    collection.delete(&[1]).expect("test: delete");
}

/// Runs `upsert_metadata` of id 1 on its own thread, paused once it knows
/// the id stores a vector (`write_hooks`), then `racing` on another thread
/// for at most `window`, then resumes the payload write. Returns whether
/// `racing` finished inside the pause.
fn race_a_row_payload_write(
    collection: &Collection,
    window: std::time::Duration,
    racing: impl FnOnce(&Collection) + Send + 'static,
) -> bool {
    use std::sync::mpsc;
    use std::time::Duration;
    let (paused_tx, paused_rx) = mpsc::channel::<()>();
    let (resume_tx, resume_rx) = mpsc::channel::<()>();
    let writer = collection.clone();
    let writer = std::thread::spawn(move || {
        super::write_hooks::pause_after_node_membership(move || {
            paused_tx.send(()).expect("test: paused");
            let _ = resume_rx.recv_timeout(Duration::from_secs(10));
        });
        writer
            .upsert_metadata([Point::metadata_only(1, json!({"v": 2}))])
            .expect("test: the payload write");
    });
    paused_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("test: the payload write paused");
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let other = collection.clone();
    let other = std::thread::spawn(move || {
        racing(&other);
        let _ = done_tx.send(());
    });
    let finished = done_rx.recv_timeout(window).is_ok();
    resume_tx.send(()).expect("test: resume");
    writer.join().expect("test: the payload write");
    other.join().expect("test: the racing write");
    finished
}

/// A vector collection's payload write holds no vector guard: once it knows
/// its ids store vectors, it holds their node locks instead, so a vector
/// write of another id — and every search a writer queued on
/// `vector_storage` would hold up — runs while it stores and syncs.
#[test]
fn a_row_payload_write_holds_up_no_vector_write() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    collection
        .upsert([Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: row");
    let finished = race_a_row_payload_write(
        &collection,
        std::time::Duration::from_secs(5),
        |collection| {
            collection
                .upsert([Point::new(2, vec![0.0, 1.0], None)])
                .expect("test: the vector write");
        },
    );
    assert!(finished, "the vector write waited for the payload write");
    assert_eq!(collection.len(), 2);
}

/// A delete of the row a payload write is about to store into waits for it:
/// both hold the row's node lock, so the payload the write stores belongs to
/// a row, and the delete then removes both. Without the delete's node lock
/// it would remove the vector inside the pause, and the payload write would
/// leave a payload no row owns.
#[test]
fn a_delete_waits_for_a_payload_write_of_its_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    collection
        .upsert([Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: row");
    race_a_row_payload_write(
        &collection,
        std::time::Duration::from_millis(500),
        |collection| collection.delete(&[1]).expect("test: the delete"),
    );
    assert!(stored_ids(&collection).is_empty(), "no payload left behind");
    assert_eq!(collection.len(), 0);
}

/// Runs `write` on its own thread, paused between the two stores of its row
/// write (`write_hooks::pause_between_row_stores`), then a delete of id 1 on
/// another thread for at most `window`, then resumes the write. Returns
/// whether the delete finished inside the pause.
fn race_a_delete_with_a_row_write(
    collection: &Collection,
    window: std::time::Duration,
    write: impl FnOnce(&Collection) + Send + 'static,
) -> bool {
    use std::sync::mpsc;
    use std::time::Duration;
    let (paused_tx, paused_rx) = mpsc::channel::<()>();
    let (resume_tx, resume_rx) = mpsc::channel::<()>();
    let writer = collection.clone();
    let writer = std::thread::spawn(move || {
        super::write_hooks::pause_between_row_stores(move || {
            paused_tx.send(()).expect("test: paused");
            let _ = resume_rx.recv_timeout(Duration::from_secs(10));
        });
        write(&writer);
    });
    paused_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("test: the row write paused");
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let deleter = collection.clone();
    let deleter = std::thread::spawn(move || {
        deleter.delete(&[1]).expect("test: the delete");
        let _ = done_tx.send(());
    });
    let finished = done_rx.recv_timeout(window).is_ok();
    resume_tx.send(()).expect("test: resume");
    writer.join().expect("test: the row write");
    deleter.join().expect("test: the delete");
    finished
}

/// What a delete racing a row write of id 1 must leave: nothing — no
/// vector, no payload, no BM25 document — the delete having waited for the
/// whole write.
fn assert_the_delete_came_last(collection: &Collection, finished_inside: bool) {
    assert!(
        !finished_inside,
        "the delete ran between the row's two stores"
    );
    assert!(stored_ids(collection).is_empty(), "a store kept id 1");
    assert_eq!(collection.len(), 0);
    assert!(ids(&collection.text_search("secret", 10).expect("test: search")).is_empty());
}

/// An upsert stores a row's payload, then its vector, under the node locks
/// of its ids, which a delete takes too: a delete of the row lands wholly
/// before or wholly after the write — here after, so both stores and BM25
/// end empty. Without the locks the delete removed the payload between the
/// two stores and the vector stored next was a row with no payload, its
/// BM25 document indexed for a deleted payload.
#[test]
fn a_delete_racing_an_upsert_of_its_row_waits_for_both_stores() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    let finished = race_a_delete_with_a_row_write(
        &collection,
        std::time::Duration::from_millis(500),
        |collection| {
            collection
                .upsert([Point::new(
                    1,
                    vec![1.0, 0.0],
                    Some(json!({"content": "secret"})),
                )])
                .expect("test: the upsert");
        },
    );
    assert_the_delete_came_last(&collection, finished);
}

/// A bulk upsert follows the same rule: its two stores and its BM25 change
/// run under the node locks of its ids, so a delete of one of them waits
/// for all of it, and no payload is left that no row owns, for a later
/// payload-less bulk write of the id to bring back.
#[test]
fn a_delete_racing_a_bulk_upsert_of_its_row_waits_for_both_stores() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    let finished = race_a_delete_with_a_row_write(
        &collection,
        std::time::Duration::from_millis(500),
        |collection| {
            collection
                .upsert_bulk(&[Point::new(
                    1,
                    vec![1.0, 0.0],
                    Some(json!({"content": "secret"})),
                )])
                .expect("test: the bulk upsert");
        },
    );
    assert_the_delete_came_last(&collection, finished);
    collection
        .upsert_bulk(&[Point::new(1, vec![1.0, 0.0], None)])
        .expect("test: a payload-less bulk write");
    let row = collection.get(&[1])[0].clone().expect("test: the row");
    assert_eq!(row.payload, None, "no payload brought back");
}

/// `upsert_metadata` writes payloads only: on a vector collection a point
/// carrying a dense vector is refused (`VELES-014`), the whole batch before
/// anything is written, instead of storing the payload and dropping the
/// vector without a word.
#[test]
fn a_metadata_write_carrying_a_vector_is_refused() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = vectors(dir.path());
    collection
        .upsert([
            Point::new(1, vec![1.0, 0.0], None),
            Point::new(2, vec![0.0, 1.0], None),
        ])
        .expect("test: rows");

    let err = collection
        .upsert_metadata([
            Point::metadata_only(1, json!({"v": 1})),
            Point::new(2, vec![1.0, 0.0], Some(json!({"v": 2}))),
        ])
        .expect_err("test: a vector refused");

    assert_eq!(err.code(), "VELES-014", "{err}");
    let rows = collection.get(&[1, 2]);
    assert!(
        rows.iter().flatten().all(|row| row.payload.is_none()),
        "nothing written"
    );
    assert_eq!(
        rows[1].as_ref().map(|row| row.vector.clone()),
        Some(vec![0.0, 1.0]),
        "the stored vector kept"
    );
}
