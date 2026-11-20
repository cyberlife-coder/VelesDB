//! Tests for `change_log`: which ids the writes after a revision changed.

use super::ChangeLog;

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
    log.record(0..ChangeLog::CAPACITY as u64);
    assert_eq!(
        log.changed_since(seen).map(|ids| ids.len()),
        Some(ChangeLog::CAPACITY)
    );
    log.record([9]);
    assert_eq!(log.changed_since(seen), None);
    assert_eq!(log.changed_since(log.revision() - 1), Some(vec![9]));
}
