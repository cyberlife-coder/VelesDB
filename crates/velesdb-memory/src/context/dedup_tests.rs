//! Unit tests for duplicate detection.

use super::*;

/// No fragment carries media — the pre-US-009 shape every existing test
/// exercises.
fn no_media(len: usize) -> Vec<Option<u64>> {
    vec![None; len]
}

/// No fragment is supersession-flagged — the shape every test that isn't
/// specifically exercising media re-anchoring (US-009, PR3) wants.
fn not_superseded(len: usize) -> Vec<bool> {
    vec![false; len]
}

#[test]
fn test_find_duplicates_exact_marks_second_occurrence() {
    let verdicts = find_duplicates(
        &["a fact", "other", "a fact"],
        true,
        &no_media(3),
        &not_superseded(3),
    );
    assert!(verdicts[0].is_none());
    assert!(verdicts[1].is_none());
    let dup = verdicts[2].expect("third entry duplicates the first");
    assert_eq!(dup.kind, DupKind::Exact);
    assert_eq!(dup.kept_seq, 0);
}

#[test]
fn test_find_duplicates_near_matches_case_and_spacing() {
    let verdicts = find_duplicates(
        &["The Server restarts.", "the  server   restarts."],
        true,
        &no_media(2),
        &not_superseded(2),
    );
    let dup = verdicts[1].expect("second entry near-duplicates the first");
    assert_eq!(dup.kind, DupKind::Near);
    assert_eq!(dup.kept_seq, 0);
}

#[test]
fn test_find_duplicates_near_detection_can_be_disabled() {
    let verdicts = find_duplicates(
        &["The Server restarts.", "the  server   restarts."],
        false,
        &no_media(2),
        &not_superseded(2),
    );
    assert!(verdicts[1].is_none(), "near detection was disabled");
}

#[test]
fn test_find_duplicates_exact_still_found_when_near_disabled() {
    let verdicts = find_duplicates(&["same", "same"], false, &no_media(2), &not_superseded(2));
    assert_eq!(verdicts[1].expect("exact duplicate").kind, DupKind::Exact);
}

#[test]
fn test_find_duplicates_distinct_contents_are_all_kept() {
    let verdicts = find_duplicates(
        &["alpha", "beta", "gamma"],
        true,
        &no_media(3),
        &not_superseded(3),
    );
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
    let verdicts = find_duplicates(
        &["Hello World", "hello  world", "hello  world"],
        true,
        &no_media(3),
        &not_superseded(3),
    );
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
        &no_media(4),
        &not_superseded(4),
    );
    assert_eq!(verdicts[3].expect("near dup").kept_seq, 0);
}

#[test]
fn test_find_duplicates_chain_points_to_first_occurrence() {
    let verdicts = find_duplicates(&["x", "x", "x"], true, &no_media(3), &not_superseded(3));
    assert_eq!(verdicts[1].expect("dup").kept_seq, 0);
    assert_eq!(verdicts[2].expect("dup").kept_seq, 0);
}

// --- Media dedup (US-009, PR1): identity is the raw decoded bytes, never
// the caption text, and never near-duplicated. ---

#[test]
fn test_find_duplicates_media_with_identical_raw_hash_is_exact_duplicate() {
    let media = vec![Some(42_u64), Some(42_u64)];
    let verdicts = find_duplicates(&["", ""], true, &media, &not_superseded(2));
    let dup = verdicts[1].expect("same raw hash duplicates the first");
    assert_eq!(dup.kind, DupKind::Exact);
    assert_eq!(dup.kept_seq, 0);
}

#[test]
fn test_find_duplicates_media_with_different_raw_hash_is_not_a_duplicate() {
    let media = vec![Some(1_u64), Some(2_u64)];
    let verdicts = find_duplicates(
        &["same caption", "same caption"],
        true,
        &media,
        &not_superseded(2),
    );
    assert!(
        verdicts[1].is_none(),
        "different images must never dedup on caption text alone"
    );
}

#[test]
fn test_find_duplicates_media_fragments_with_blank_captions_never_false_positive_dedup() {
    // Two distinct screenshots, both with the empty caption that is typical
    // for a bare screenshot: under plain text-content dedup these would be
    // an exact match ("" == ""); media identity must ignore that entirely.
    let media = vec![Some(111_u64), Some(222_u64)];
    let verdicts = find_duplicates(&["", ""], true, &media, &not_superseded(2));
    assert!(verdicts[1].is_none());
}

#[test]
fn test_find_duplicates_media_never_marked_near_duplicate() {
    // Even a contrived setup that resembles a near-dup pattern must still
    // resolve as Exact-or-none for media — DupKind::Near never appears when
    // a media hash is present.
    let media = vec![Some(7_u64), Some(7_u64)];
    let verdicts = find_duplicates(
        &["Hello World", "hello  world"],
        true,
        &media,
        &not_superseded(2),
    );
    assert_eq!(verdicts[1].expect("dup").kind, DupKind::Exact);
}

#[test]
fn test_find_duplicates_media_fragment_does_not_pollute_text_dedup_of_others() {
    // A media fragment with a blank caption must not register "" into the
    // text-content dedup tables — a later, unrelated plain-text fragment
    // with empty content must NOT be marked a duplicate of the media
    // fragment merely because both "contents" are blank.
    let media = vec![Some(999_u64), None];
    let verdicts = find_duplicates(&["", ""], true, &media, &not_superseded(2));
    assert!(
        verdicts[1].is_none(),
        "a plain-text empty fragment must not dedup against a media fragment's blank caption"
    );
}

#[test]
fn test_find_duplicates_media_and_text_dedup_use_independent_namespaces() {
    // Fragment #0 is plain text "shared", #1 is media whose caption also
    // happens to be "shared" but with a distinct raw hash — must not dedup
    // against #0 via the text namespace.
    let media = vec![None, Some(1_u64)];
    let verdicts = find_duplicates(&["shared", "shared"], true, &media, &not_superseded(2));
    assert!(verdicts[1].is_none());
}

#[test]
fn test_find_duplicates_media_chain_points_to_first_occurrence() {
    let media = vec![Some(5_u64), Some(5_u64), Some(5_u64)];
    let verdicts = find_duplicates(&["", "", ""], true, &media, &not_superseded(3));
    assert_eq!(verdicts[1].expect("dup").kept_seq, 0);
    assert_eq!(verdicts[2].expect("dup").kept_seq, 0);
}
