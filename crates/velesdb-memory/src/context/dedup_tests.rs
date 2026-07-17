//! Unit tests for duplicate detection.

use super::*;

#[test]
fn test_find_duplicates_exact_marks_second_occurrence() {
    let verdicts = find_duplicates(&["a fact", "other", "a fact"], true);
    assert!(verdicts[0].is_none());
    assert!(verdicts[1].is_none());
    let dup = verdicts[2].expect("third entry duplicates the first");
    assert_eq!(dup.kind, DupKind::Exact);
    assert_eq!(dup.kept_seq, 0);
}

#[test]
fn test_find_duplicates_near_matches_case_and_spacing() {
    let verdicts = find_duplicates(&["The Server restarts.", "the  server   restarts."], true);
    let dup = verdicts[1].expect("second entry near-duplicates the first");
    assert_eq!(dup.kind, DupKind::Near);
    assert_eq!(dup.kept_seq, 0);
}

#[test]
fn test_find_duplicates_near_detection_can_be_disabled() {
    let verdicts = find_duplicates(&["The Server restarts.", "the  server   restarts."], false);
    assert!(verdicts[1].is_none(), "near detection was disabled");
}

#[test]
fn test_find_duplicates_exact_still_found_when_near_disabled() {
    let verdicts = find_duplicates(&["same", "same"], false);
    assert_eq!(verdicts[1].expect("exact duplicate").kind, DupKind::Exact);
}

#[test]
fn test_find_duplicates_distinct_contents_are_all_kept() {
    let verdicts = find_duplicates(&["alpha", "beta", "gamma"], true);
    assert!(verdicts.iter().all(Option::is_none));
}

#[test]
fn test_find_duplicates_exact_copy_of_a_near_duplicate_reports_exact() {
    // "hello  world" (#1) is only a NEAR duplicate of "Hello World" (#0) —
    // its bytes differ (case, spacing). #2 is a byte-identical copy of #1,
    // not of #0: kept_seq must point at #1, the fragment #2 is actually
    // byte-identical to, never at the root #0 it merely resembles. Pointing
    // at #0 would let downstream code (`dup_verdict`) assume #2's exact
    // bytes survive whenever #0 is emitted verbatim — false whenever #0 and
    // #1/#2 differ, which is exactly why they were only a *near* match.
    let verdicts = find_duplicates(&["Hello World", "hello  world", "hello  world"], true);
    let dup = verdicts[2].expect("third entry duplicates the second");
    assert_eq!(
        dup.kind,
        DupKind::Exact,
        "a byte-identical copy must be recorded as an exact duplicate"
    );
    assert_eq!(
        dup.kept_seq, 1,
        "the byte-identical twin is #1, not the near-duplicate root #0"
    );
}

#[test]
fn test_find_duplicates_near_dup_still_anchors_the_near_chain_at_the_root() {
    // Even though an exact-duplicate chain now anchors at the nearest byte-
    // identical twin (previous test), the *near*-duplicate chain must still
    // anchor every near match at the true root: a fourth, distinct-again
    // near-duplicate of "Hello World" must point at #0, not at #1.
    let verdicts = find_duplicates(
        &["Hello World", "hello  world", "hello  world", "HELLO WORLD"],
        true,
    );
    assert_eq!(verdicts[3].expect("near dup").kept_seq, 0);
}

#[test]
fn test_find_duplicates_chain_points_to_first_occurrence() {
    let verdicts = find_duplicates(&["x", "x", "x"], true);
    assert_eq!(verdicts[1].expect("dup").kept_seq, 0);
    assert_eq!(verdicts[2].expect("dup").kept_seq, 0);
}
