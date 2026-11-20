//! A ranked walk raced by writes: what a filtered walk's prefilter bitmap
//! does not describe is offered again, and a walk always ends however its
//! writer keeps changing the rows it kept.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::json;

use super::ranked::{on_guard_release, RESTARTS, WALK_BATCH};
use super::ranked_hooks::on_prefilter_built;
use crate::collection::core::write_hooks::{pause_after_index_visible, pause_after_row_write};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::filter::{Condition, Filter};
use crate::point::{Point, SearchResult};

fn ids(results: &[SearchResult]) -> Vec<u64> {
    results.iter().map(|r| r.point.id).collect()
}

/// Row `id` holding "common", in category `cat`: 1 the best hit, 2 the
/// second, every other one tied below them, a tie by ascending id.
fn row(id: u64, cat: &str) -> Point {
    let content = match id {
        1 => "common common common",
        2 => "common common",
        _ => "common",
    };
    Point::new(
        id,
        vec![1.0, 0.5],
        Some(json!({"content": content, "cat": cat})),
    )
}

/// `n` rows ([`row`]), those of `matches` in category `b`, every other one
/// in `a`.
fn rows(dir: &std::path::Path, n: u64, matches: &[u64]) -> Collection {
    let collection =
        Collection::create(dir.join("r"), 2, DistanceMetric::Cosine).expect("test: collection");
    collection
        .upsert((1..=n).map(|id| row(id, if matches.contains(&id) { "b" } else { "a" })))
        .expect("test: upsert");
    collection
}

fn in_b() -> Filter {
    Filter::new(Condition::eq("cat", "b"))
}

/// A write between the prefilter bitmap and the walk's first hold — 1, the
/// best hit, moved into the filter's category — is one the bitmap does not
/// describe: the walk offers 1 too, and answers the state its hold reads.
#[test]
fn a_write_after_the_prefilter_and_before_the_first_hold_is_offered() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = rows(dir.path(), 20, &[20]);
    collection.create_index("cat").expect("test: index");
    let writer = collection.clone();
    on_prefilter_built(move || {
        writer.upsert([row(1, "b")]).expect("test: 1 moved into b");
    });

    let hits = collection
        .text_search_with_filter("common", 1, &in_b())
        .expect("test: text search");

    assert_eq!(ids(&hits), [1]);
}

/// What a filtered walk, run by `search`, answers while a row write that
/// moved 1 into category `b` has released its storage guards and not yet
/// returned.
fn searched_after_the_write(
    collection: &Collection,
    write: impl FnOnce(&Collection),
    search: impl FnOnce(&Collection) -> Vec<SearchResult> + 'static,
) -> Vec<u64> {
    let seen = Rc::new(RefCell::new(None));
    let (reader, slot) = (collection.clone(), Rc::clone(&seen));
    pause_after_row_write(move || *slot.borrow_mut() = Some(ids(&search(&reader))));
    write(collection);
    let seen = seen.borrow_mut().take();
    seen.expect("test: the write paused")
}

/// Once a write released its guards, the secondary indexes describe what it
/// stored: a walk the reader runs then — its bitmap built from them, its
/// first hold reading the stores the write left — finds the row the write
/// moved into the filter's category, whichever write it was.
#[test]
fn the_secondary_indexes_follow_a_write_before_it_releases_its_guards() {
    let search = |reader: &Collection| {
        reader
            .text_search_with_filter("common", 1, &in_b())
            .expect("test: text search")
    };
    for bulk in [false, true] {
        let dir = tempfile::tempdir().expect("test: tempdir");
        let collection = rows(dir.path(), 20, &[20]);
        collection.create_index("cat").expect("test: index");

        let seen = searched_after_the_write(
            &collection,
            |writer| {
                let moved = vec![row(1, "b")];
                if bulk {
                    writer.upsert_bulk(&moved).map(|_| ())
                } else {
                    writer.upsert(moved)
                }
                .expect("test: 1 moved into b");
            },
            search,
        );

        assert_eq!(seen, [1], "bulk: {bulk}");
    }
}

/// A writer that, between every two holds, moves 1 and 2 — two rows the
/// walk keeps — in and out of the filter's category, one leaving as the
/// other comes back, up to `cap` times; how many times the walk released
/// its guards, read through `releases`.
fn toggle_kept_rows(collection: &Collection, cap: u32) -> (Rc<Cell<u32>>, Rc<Cell<bool>>) {
    let releases = Rc::new(Cell::new(0_u32));
    let active = Rc::new(Cell::new(true));
    let (writer, seen, on) = (collection.clone(), Rc::clone(&releases), Rc::clone(&active));
    on_guard_release(move || {
        if !on.get() || seen.get() >= cap {
            return;
        }
        seen.set(seen.get() + 1);
        let (first, second) = if seen.get() % 2 == 1 {
            ("a", "b")
        } else {
            ("b", "a")
        };
        writer
            .upsert([row(1, first), row(2, second)])
            .expect("test: 1 and 2 toggled");
    });
    (releases, active)
}

/// A walk whose writer keeps changing the rows it kept between every two
/// holds still ends: a row dropped under the hold that would finish is
/// replaced under that same hold. 1 and 2 are kept under the first hold;
/// under the third, 2100 fills the `k`, 2 has left and 2200 takes its place
/// there.
#[test]
fn a_walk_whose_kept_rows_a_writer_keeps_toggling_ends() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 3 * WALK_BATCH as u64;
    let collection = rows(dir.path(), n, &[1, 2, 2100, 2200]);
    let (releases, active) = toggle_kept_rows(&collection, 60);

    let hits = collection
        .text_search_with_filter("common", 3, &in_b())
        .expect("test: text search");

    active.set(false);
    let state = collection
        .text_search_with_filter("common", 3, &in_b())
        .expect("test: text search");
    assert!(
        releases.get() <= 2 + RESTARTS,
        "{} releases: the walk did not stop releasing",
        releases.get()
    );
    assert_eq!(ids(&hits), ids(&state));
}

/// A walk whose dropped rows a writer keeps bringing back while another
/// leaves, and whose replacement lies past what a hold offers, counts each
/// such failed finish: after [`RESTARTS`] of them it offers every candidate
/// it needs under one hold, and ends. 1, 2 and 3062 fill the `k` at the end
/// of the third batch; past them lie seven batches of rows the filter
/// refuses.
#[test]
fn a_walk_counts_the_finishes_a_writer_keeps_failing() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let n = 10 * WALK_BATCH as u64;
    let deep = 3 * WALK_BATCH as u64 - 10;
    let collection = rows(dir.path(), n, &[1, 2, deep]);
    let (releases, active) = toggle_kept_rows(&collection, 60);

    let hits = collection
        .text_search_with_filter("common", 3, &in_b())
        .expect("test: text search");

    active.set(false);
    let state = collection
        .text_search_with_filter("common", 3, &in_b())
        .expect("test: text search");
    assert!(
        releases.get() <= 2 + RESTARTS,
        "{} releases: the walk did not count its failed finishes",
        releases.get()
    );
    assert_eq!(ids(&hits), ids(&state));
}

/// A secondary index is visible to a filtered walk only once it holds every
/// row: built empty and filled after, it gave the walk's prefilter an empty
/// bitmap, which set every candidate aside, and the walk returned nothing.
#[test]
fn a_secondary_index_is_seen_only_once_it_holds_every_row() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let collection = rows(dir.path(), 20, &[3, 20]);
    let seen = Rc::new(RefCell::new(None));
    let (reader, slot) = (collection.clone(), Rc::clone(&seen));
    pause_after_index_visible(move || {
        let hits = reader
            .text_search_with_filter("common", 2, &in_b())
            .expect("test: text search");
        *slot.borrow_mut() = Some(ids(&hits));
    });

    collection.create_index("cat").expect("test: index");

    assert_eq!(seen.borrow_mut().take(), Some(vec![3, 20]));
}
