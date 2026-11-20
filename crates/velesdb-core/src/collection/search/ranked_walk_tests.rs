//! A ranked walk releases the storage guards between batches, so a writer
//! never waits for the whole walk, and judges again under its last hold the
//! rows an earlier hold admitted.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::json;

use super::{on_guard_release, WALK_BATCH};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// `n` rows holding "common": 1 the best hit, category `b`; `n - 1` and `n`
/// the two worst, category `b`; every other one between them, category `a`.
fn collection(dir: &std::path::Path, n: u64) -> Collection {
    let collection =
        Collection::create(dir.join("w"), 2, DistanceMetric::Cosine).expect("test: collection");
    collection
        .upsert((1..=n).map(|id| {
            let (content, cat) = match id {
                1 => ("common common common", "b"),
                _ if id == n - 1 => ("common filler", "b"),
                _ if id == n => ("common filler filler", "b"),
                _ => ("common", "a"),
            };
            Point::new(
                id,
                vec![1.0, 0.5],
                Some(json!({"content": content, "cat": cat})),
            )
        }))
        .expect("test: upsert");
    collection
}

/// A filter refusing every candidate makes the walk offer all of them: it
/// releases the guards between every two batches, and a writer can take
/// them there — the walk never holds them across the whole collection.
#[test]
fn a_refusing_walk_lets_a_writer_in_between_its_batches() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    let written = Rc::new(Cell::new(0_u64));
    let (writer, count) = (collection.clone(), Rc::clone(&written));
    on_guard_release(move || {
        // No guard held here, so this thread's own write takes the storage
        // write guards — it would deadlock under a read guard still held.
        assert_eq!(crate::lock_audit::guards_held(), 0, "a guard is held");
        let id = 1_000_000 + count.get();
        writer
            .upsert([Point::new(id, vec![1.0, 0.5], Some(json!({"cat": "w"})))])
            .expect("test: a write between two batches");
        count.set(count.get() + 1);
    });

    let hits = collection
        .text_search_with_filter("common", 10, &Filter::new(Condition::eq("cat", "c")))
        .expect("test: text search");

    assert!(hits.is_empty(), "{:?}", ids(&hits));
    assert!(
        written.get() >= 2,
        "{} writes went in between the batches of a walk over {n} candidates",
        written.get()
    );
    assert_eq!(
        collection.get(&[1_000_000]).into_iter().flatten().count(),
        1
    );
}

/// A row an earlier hold admitted and a writer deleted between two holds is
/// dropped, and the walk goes on to the next match instead.
#[test]
fn a_row_deleted_between_two_holds_is_judged_again_and_replaced() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    let writer = collection.clone();
    let deleted = Cell::new(false);
    on_guard_release(move || {
        if !deleted.replace(true) {
            writer.delete(&[1]).expect("test: delete");
        }
    });

    let hits = collection
        .text_search_with_filter("common", 2, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [n - 1, n]);
}

/// A filter the secondary indexes answer rules its refused candidates out
/// before the walk takes any guard: the same refusing walk over an indexed
/// field offers nothing, so it never has to release the guards.
#[test]
fn an_indexed_filter_rules_candidates_out_before_the_walk() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    collection.create_index("cat").expect("test: index");
    let released = Rc::new(Cell::new(0_u32));
    let seen = Rc::clone(&released);
    on_guard_release(move || seen.set(seen.get() + 1));

    let hits = collection
        .text_search_with_filter("common", 10, &Filter::new(Condition::eq("cat", "c")))
        .expect("test: text search");
    let matching = collection
        .text_search_with_filter("common", 2, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert!(hits.is_empty(), "{:?}", ids(&hits));
    assert_eq!(ids(&matching), [1, n - 1]);
    assert_eq!(released.get(), 0, "the walk released its guards");
}

/// A candidate refused under an early hold and made admissible by a write
/// before a later one is offered again: the walk answers the exact top `k`
/// of the state its last hold reads. Two serial writes land between the
/// first two holds — 1, the best hit, refused (category `a`), becomes `b`,
/// then 2000, in the second batch, becomes `b` too — and the only answers a
/// state of the collection gives are `[n]` (before both) and `[1]` (after
/// either); a walk that never offered 1 again answered `[2000]`.
#[test]
fn a_candidate_refused_before_a_write_is_offered_again() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    collection
        .upsert([Point::new(
            1,
            vec![1.0, 0.5],
            Some(json!({"content": "common common common", "cat": "a"})),
        )])
        .expect("test: 1 refused at first");
    collection
        .upsert([Point::new(
            n - 1,
            vec![1.0, 0.5],
            Some(json!({"content": "common filler", "cat": "a"})),
        )])
        .expect("test: only n matches at first");
    let writer = collection.clone();
    let written = Cell::new(false);
    on_guard_release(move || {
        if !written.replace(true) {
            writer
                .upsert([Point::new(
                    1,
                    vec![1.0, 0.5],
                    Some(json!({"content": "common common common", "cat": "b"})),
                )])
                .expect("test: first write");
            writer
                .upsert([Point::new(
                    2000,
                    vec![1.0, 0.5],
                    Some(json!({"content": "common", "cat": "b"})),
                )])
                .expect("test: second write");
        }
    });

    let hits = collection
        .text_search_with_filter("common", 1, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [1]);
}

/// A candidate the filter's prefilter bitmap set aside before the walk — 1,
/// category `a` when the walk began — and that a write between two holds
/// makes admissible is offered too, once the walk starts over: the bitmap
/// described a state the walk no longer reads.
#[test]
fn a_candidate_the_prefilter_set_aside_is_offered_after_a_write() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection =
        Collection::create(dir.path().join("p"), 2, DistanceMetric::Cosine).expect("test: c");
    collection
        .upsert((1..=n).map(|id| {
            let (content, cat, flag) = match id {
                1 => ("common common common", "a", 0),
                _ if id == n => ("common filler filler", "b", 0),
                _ => ("common", "b", 1),
            };
            Point::new(
                id,
                vec![1.0, 0.5],
                Some(json!({"content": content, "cat": cat, "flag": flag})),
            )
        }))
        .expect("test: upsert");
    collection.create_index("cat").expect("test: index");
    let writer = collection.clone();
    let written = Cell::new(false);
    on_guard_release(move || {
        if !written.replace(true) {
            writer
                .upsert([Point::new(
                    1,
                    vec![1.0, 0.5],
                    Some(json!({"content": "common common common", "cat": "b", "flag": 0})),
                )])
                .expect("test: write");
        }
    });
    let filter = Filter::new(Condition::and(vec![
        Condition::eq("cat", "b"),
        Condition::eq("flag", 0),
    ]));

    let hits = collection
        .text_search_with_filter("common", 1, &filter)
        .expect("test: text search");

    assert_eq!(ids(&hits), [1]);
}

/// Upserts more new rows than a store's change log keeps, ids from `from`.
fn flood(collection: &Collection, from: u64) {
    let rows = crate::storage::ChangeLog::CAPACITY as u64 + 1;
    collection
        .upsert(
            (from..from + rows).map(|id| Point::new(id, vec![1.0, 0.5], Some(json!({"cat": "w"})))),
        )
        .expect("test: a flood of writes between two holds");
}

/// More writes between two holds than the change logs keep make the walk
/// start over — it can no longer tell which candidates they changed — and
/// still answer the state its last hold reads: 1, refused at first and made
/// admissible in that flood, is found.
#[test]
fn a_walk_starts_over_when_the_writes_overflow_the_change_log() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    collection
        .upsert([Point::new(
            1,
            vec![1.0, 0.5],
            Some(json!({"content": "common common common", "cat": "a"})),
        )])
        .expect("test: 1 refused at first");
    let writer = collection.clone();
    let written = Cell::new(false);
    on_guard_release(move || {
        if !written.replace(true) {
            writer
                .upsert([Point::new(
                    1,
                    vec![1.0, 0.5],
                    Some(json!({"content": "common common common", "cat": "b"})),
                )])
                .expect("test: 1 made admissible");
            flood(&writer, 1_000_000);
        }
    });

    let hits = collection
        .text_search_with_filter("common", 1, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [1]);
}

/// Floods of writes that keep overflowing the change logs between the holds
/// make the walk start over a bounded number of times; it then finishes
/// under one hold, so it always ends — with the exact answer of the state
/// that hold reads.
#[test]
fn a_walk_raced_by_writes_without_end_still_finishes() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    let releases = Rc::new(Cell::new(0_u32));
    let (writer, seen) = (collection.clone(), Rc::clone(&releases));
    on_guard_release(move || {
        seen.set(seen.get() + 1);
        flood(&writer, 1_000_000 * u64::from(seen.get()));
    });

    let hits = collection
        .text_search_with_filter("common", 1, &Filter::new(Condition::eq("cat", "c")))
        .expect("test: text search");

    assert!(hits.is_empty(), "{:?}", ids(&hits));
    assert!(
        releases.get() <= 2 * (super::RESTARTS + 1),
        "{} releases: the walk did not stop starting over",
        releases.get()
    );
}

/// Writes that keep landing between the holds, each few enough for the
/// change logs, never make the walk start over nor hold the guards for more
/// than a batch: it judges again only what they changed.
#[test]
fn a_walk_raced_by_small_writes_keeps_its_batches() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    let releases = Rc::new(Cell::new(0_u64));
    let (writer, seen) = (collection.clone(), Rc::clone(&releases));
    on_guard_release(move || {
        seen.set(seen.get() + 1);
        writer
            .upsert([Point::new(
                1_000_000 + seen.get(),
                vec![1.0, 0.5],
                Some(json!({"cat": "w"})),
            )])
            .expect("test: a write between two holds");
    });

    let hits = collection
        .text_search_with_filter("common", 10, &Filter::new(Condition::eq("cat", "c")))
        .expect("test: text search");

    assert!(hits.is_empty(), "{:?}", ids(&hits));
    // One release between every two of the walk's batches, none more: no
    // restart, and no hold that offered more than a batch.
    assert_eq!(releases.get(), n.div_ceil(WALK_BATCH as u64) - 1);
}

/// A kept row is judged once per hold: its admission under the hold that
/// finishes the walk is not run again by the hydration.
#[test]
fn a_kept_row_is_judged_once_under_the_finishing_hold() {
    use crate::collection::search::query::row_universe::RowUniverse;
    use std::collections::HashMap;
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = collection(dir.path(), 8);
    let judged = std::cell::RefCell::new(HashMap::<u64, u32>::new());

    let hits = collection.ranked_hits(
        super::Ranked::new(vec![(1, 3.0), (2, 2.0), (3, 1.0)]),
        2,
        RowUniverse::Vectors,
        |_| true,
        |id, _| {
            *judged.borrow_mut().entry(id).or_default() += 1;
            true
        },
    );

    assert_eq!(ids(&hits), [1, 2]);
    assert_eq!(judged.into_inner(), HashMap::from([(1, 1), (2, 1)]));
}

/// A write to the payload store alone — `upsert_metadata`, which leaves the
/// vectors as they are — moves the stores' revision too: the candidate it
/// makes admissible between two holds is offered again.
#[test]
fn a_payload_only_write_between_holds_makes_the_walk_start_over() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    collection
        .upsert_metadata([Point::metadata_only(
            1,
            json!({"content": "common common common", "cat": "a"}),
        )])
        .expect("test: 1 refused at first");
    let writer = collection.clone();
    let written = Cell::new(false);
    on_guard_release(move || {
        if !written.replace(true) {
            writer
                .upsert_metadata([Point::metadata_only(
                    1,
                    json!({"content": "common common common", "cat": "b"}),
                )])
                .expect("test: the payload write");
        }
    });

    let hits = collection
        .text_search_with_filter("common", 1, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [1]);
}

/// A row an earlier hold kept that expires before the walk's last hold — no
/// write in between, so the walk does not start over — is judged again under
/// that hold and dropped, and the walk goes on to the next match.
#[test]
fn a_kept_row_that_expires_between_two_holds_is_dropped() {
    use crate::collection::expiry::now_unix_secs;
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    // At least a second away, so the first hold still admits 1.
    let expires_at = now_unix_secs() + 2;
    collection
        .upsert([Point::new(
            1,
            vec![1.0, 0.5],
            Some(json!({"content": "common common common", "cat": "b", "_veles_expires_at": expires_at})),
        )])
        .expect("test: 1 expires soon");
    on_guard_release(move || {
        // No write: the walk's next hold reads the same revision, and only
        // time has changed whether 1 passes.
        while now_unix_secs() < expires_at {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });

    let hits = collection
        .text_search_with_filter("common", 2, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [n - 1, n]);
}

/// Rows a write makes admissible above the rows already kept take their
/// places, and the worst kept goes back among the candidates left — for a
/// later drop to bring it back. 2500 and 2600 are the only matches the first
/// three batches find, 2500 about to expire; a write then makes 1 and 2
/// match: they displace 2600, 2500 expires, and 2600 comes back.
#[test]
fn rows_a_write_admits_above_the_kept_displace_the_worst() {
    use crate::collection::expiry::now_unix_secs;
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 4 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    // Seconds away, so the third hold still admits 2500.
    let expires_at = now_unix_secs() + 3;
    let row = move |id: u64, cat: &str| {
        let content = if id == 1 {
            "common common common"
        } else {
            "common"
        };
        let mut payload = json!({"content": content, "cat": cat});
        if id == 2500 {
            payload["_veles_expires_at"] = json!(expires_at);
        }
        Point::new(id, vec![1.0, 0.5], Some(payload))
    };
    collection
        .upsert([
            row(1, "a"),
            row(2500, "b"),
            row(2600, "b"),
            row(n - 1, "a"),
            row(n, "a"),
        ])
        .expect("test: 2500 and 2600 match at first");
    let writer = collection.clone();
    let releases = Cell::new(0_u32);
    on_guard_release(move || {
        releases.set(releases.get() + 1);
        if releases.get() == 3 {
            writer
                .upsert([row(1, "b"), row(2, "b")])
                .expect("test: 1 and 2 match");
            while now_unix_secs() < expires_at {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    });

    let hits = collection
        .text_search_with_filter("common", 3, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [1, 2, 2600]);
}

/// The walk returns exactly `k`, the best: a row a write admits above the
/// kept ones takes a place even when the `k` are full, and the worst kept
/// drops out. 2500 is the only match the first three batches find; a write
/// then makes 1 and 2 match.
#[test]
fn a_full_walk_takes_the_better_rows_a_write_admits() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 4 * WALK_BATCH as u64;
    let collection = collection(dir.path(), n);
    let row = |id: u64, cat: &str| {
        let content = if id == 1 {
            "common common common"
        } else {
            "common"
        };
        Point::new(
            id,
            vec![1.0, 0.5],
            Some(json!({"content": content, "cat": cat})),
        )
    };
    collection
        .upsert([row(1, "a"), row(2500, "b"), row(n - 1, "a"), row(n, "a")])
        .expect("test: only 2500 matches at first");
    let writer = collection.clone();
    let releases = Cell::new(0_u32);
    on_guard_release(move || {
        releases.set(releases.get() + 1);
        if releases.get() == 3 {
            writer
                .upsert([row(1, "b"), row(2, "b")])
                .expect("test: 1 and 2 match");
        }
    });

    let hits = collection
        .text_search_with_filter("common", 2, &Filter::new(Condition::eq("cat", "b")))
        .expect("test: text search");

    assert_eq!(ids(&hits), [1, 2]);
}
