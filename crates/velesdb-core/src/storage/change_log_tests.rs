//! Tests for `change_log`: which ids the writes after a revision changed.

use super::ChangeLog;

/// The ids of `range`, as a write hands them over: a sized batch.
fn ids(range: std::ops::Range<u64>) -> Vec<u64> {
    range.collect()
}

#[test]
fn the_ids_written_after_a_revision_are_the_changes_since_it() {
    let mut log = ChangeLog::default();
    log.record([1, 2]);
    let seen = log.revision();
    log.record([3]);
    log.record([2]);
    assert_eq!(log.changed_since(seen), Some(vec![3, 2]));
    assert_eq!(log.changed_since(log.revision()), Some(vec![]));
}

#[test]
fn a_revision_the_log_forgot_part_of_has_no_answer() {
    let mut log = ChangeLog::default();
    log.record([7]);
    let seen = log.revision();
    log.record(ids(0..ChangeLog::CAPACITY as u64));
    assert_eq!(
        log.changed_since(seen).map(|ids| ids.len()),
        Some(ChangeLog::CAPACITY)
    );
    log.record([9]);
    assert_eq!(log.changed_since(seen), None);
    assert_eq!(log.changed_since(log.revision() - 1), Some(vec![9]));
}

/// A write of more ids than the log keeps leaves the log holding the last
/// `CAPACITY` of them, in a buffer no larger than a log of `CAPACITY` ids
/// needs: the size of the largest write is not kept for the store's life.
#[test]
fn a_write_larger_than_the_log_keeps_only_its_last_ids_in_a_bounded_buffer() {
    let mut log = ChangeLog::default();
    log.record([7]);
    let before = log.revision();
    let written = 4 * ChangeLog::CAPACITY as u64;
    log.record(ids(0..written));
    let seen = log.revision();
    log.record([9]);

    assert_eq!(
        log.changed_since(before),
        None,
        "the write overflowed the log"
    );
    assert_eq!(log.changed_since(seen), Some(vec![9]));
    assert_eq!(log.entries.len(), ChangeLog::CAPACITY);
    assert_eq!(
        log.entries.front().map(|&(_, id)| id),
        Some(written - ChangeLog::CAPACITY as u64 + 1),
        "the oldest id kept"
    );
    assert!(
        log.entries.capacity() < 2 * ChangeLog::CAPACITY,
        "a buffer of {} entries kept for a log of {}",
        log.entries.capacity(),
        ChangeLog::CAPACITY
    );
}

/// A write of exactly `CAPACITY` ids forgets every earlier one: a reader of
/// the revision before it learns that it cannot tell, one of its own
/// revision sees nothing since.
#[test]
fn a_write_of_capacity_ids_forgets_the_earlier_ones() {
    let mut log = ChangeLog::default();
    log.record([7]);
    let before = log.revision();
    log.record([8]);
    let middle = log.revision();
    log.record(ids(100..100 + ChangeLog::CAPACITY as u64));

    assert_eq!(log.changed_since(before), None);
    assert_eq!(
        log.changed_since(middle).map(|ids| ids.len()),
        Some(ChangeLog::CAPACITY)
    );
    assert_eq!(log.changed_since(log.revision()), Some(vec![]));
}
