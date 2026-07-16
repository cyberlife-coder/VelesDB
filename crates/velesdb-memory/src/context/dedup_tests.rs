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
fn test_find_duplicates_chain_points_to_first_occurrence() {
    let verdicts = find_duplicates(&["x", "x", "x"], true);
    assert_eq!(verdicts[1].expect("dup").kept_seq, 0);
    assert_eq!(verdicts[2].expect("dup").kept_seq, 0);
}
