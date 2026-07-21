//! TDD suite for [`super::segment_transcript`] (V2b-2): format detection,
//! turn splitting, fence/log/body sub-segmentation, and normalization
//! (merge/re-split/caps).

use std::fmt::Write as _;

use serde_json::Value;

use super::{segment_transcript, SegmentFormat, SegmentKind, SegmentationPolicy};
use crate::error::MemoryError;
use crate::limits::{MAX_FRAGMENTS, MAX_FRAGMENT_BYTES};

fn policy() -> SegmentationPolicy {
    SegmentationPolicy::default()
}

#[test]
fn plain_text_without_markers_is_single_body_segment() {
    // Given plain prose with no role marker anywhere
    let text = "The deploy pipeline is green.\nNothing else to report.";

    // When segmenting with the default (auto) policy
    let outcome = segment_transcript(text, &policy()).expect("segments");

    // Then it detects plain format and yields exactly one body segment
    // covering the whole text, with a null role
    assert_eq!(outcome.format_detected, SegmentFormat::Plain);
    assert_eq!(outcome.segments.len(), 1);
    let segment = &outcome.segments[0];
    assert_eq!(segment.kind, SegmentKind::Body);
    assert_eq!(segment.role, None);
    assert_eq!(segment.turn, 0);
    assert_eq!(segment.fragment.content, text);
    assert_eq!(segment.byte_start, 0);
    assert_eq!(segment.byte_end, text.len());
}

#[test]
fn user_assistant_markers_split_turns() {
    // Given a two-turn plain transcript
    let text = "User: what is the deploy status?\nAssistant: it is green.\n";

    // When segmenting
    let outcome = segment_transcript(text, &policy()).expect("segments");

    // Then it splits into two turns with the expected roles, in order,
    // partitioning the transcript
    assert_eq!(outcome.format_detected, SegmentFormat::Plain);
    let roles: Vec<Option<String>> = outcome.segments.iter().map(|s| s.role.clone()).collect();
    assert_eq!(
        roles,
        vec![Some("User".to_owned()), Some("Assistant".to_owned())]
    );
    assert_eq!(outcome.segments[0].turn, 0);
    assert_eq!(outcome.segments[1].turn, 1);
    assert!(outcome.segments[0].fragment.content.starts_with("User:"));
    assert!(outcome.segments[1]
        .fragment
        .content
        .starts_with("Assistant:"));
}

#[test]
fn jsonl_roles_detected_and_forced_format_errors_on_bad_line() {
    // Given a well-formed JSONL transcript
    let good =
        "{\"role\":\"system\",\"content\":\"be terse\"}\n{\"role\":\"user\",\"content\":\"hi\"}\n";

    // When segmenting with format forced to jsonl
    let mut jsonl_policy = policy();
    jsonl_policy.format = SegmentFormat::Jsonl;
    let outcome = segment_transcript(good, &jsonl_policy).expect("valid jsonl segments");

    // Then roles come straight from the parsed JSON, one turn per line
    assert_eq!(outcome.format_detected, SegmentFormat::Jsonl);
    assert_eq!(outcome.segments.len(), 2);
    assert_eq!(outcome.segments[0].role, Some("system".to_owned()));
    assert_eq!(outcome.segments[0].fragment.content, "be terse");
    assert_eq!(outcome.segments[1].role, Some("user".to_owned()));
    assert_eq!(outcome.segments[1].fragment.content, "hi");

    // Given a transcript with one bad line
    let bad = "{\"role\":\"system\",\"content\":\"be terse\"}\nnot json at all\n";

    // When forcing jsonl on it
    let err = segment_transcript(bad, &jsonl_policy).expect_err("bad line must error");

    // Then it is a hard, actionable error — never a silent fallback to plain
    // — and it is the dedicated `SegmentationError` variant (m2, issue
    // #1516): a bad-format parse failure is not a budget breach, so it must
    // not surface as `ContextOverLimit` (whose "over limit" wording would be
    // misleading for a caller filtering on the message).
    match err {
        MemoryError::SegmentationError(msg) => {
            assert!(msg.contains("line 2"), "{msg}");
        }
        other => panic!("expected SegmentationError, got {other:?}"),
    }
}

#[test]
fn forced_jsonl_no_nonblank_line_is_also_segmentation_error() {
    // Given a "transcript" forced to jsonl that has no non-blank line at
    // all (jsonl_pieces' other failure mode, distinct from a bad line)
    let blank_only = "\n\n";
    let mut jsonl_policy = policy();
    jsonl_policy.format = SegmentFormat::Jsonl;

    // When forcing jsonl on it
    let err = segment_transcript(blank_only, &jsonl_policy).expect_err("must error");

    // Then it is still SegmentationError, not ContextOverLimit — same
    // rationale as the bad-line case above.
    match err {
        MemoryError::SegmentationError(_) => {}
        other => panic!("expected SegmentationError, got {other:?}"),
    }
}

#[test]
fn genuine_budget_breaches_still_return_context_over_limit() {
    // Non-regression (m2, issue #1516): introducing `SegmentationError` for
    // format/parsing failures must not change the variant for a REAL
    // budget/cap breach — those keep returning `ContextOverLimit` exactly
    // as before.
    use crate::limits::MAX_TRANSCRIPT_BYTES;

    // Transcript over the byte cap
    let too_big = "a".repeat(MAX_TRANSCRIPT_BYTES + 1);
    let err = segment_transcript(&too_big, &policy()).expect_err("over cap");
    assert!(
        matches!(err, MemoryError::ContextOverLimit(_)),
        "expected ContextOverLimit, got {err:?}"
    );

    // Too many fragments after merging
    let mut many_turns = String::new();
    for i in 0..=MAX_FRAGMENTS {
        writeln!(many_turns, "User: turn {i}").expect("write to String never fails");
    }
    let mut tight_policy = policy();
    tight_policy.min_segment_bytes = 0;
    let err = segment_transcript(&many_turns, &tight_policy).expect_err("over fragment cap");
    assert!(
        matches!(err, MemoryError::ContextOverLimit(_)),
        "expected ContextOverLimit, got {err:?}"
    );

    // An unsplittable oversized fence
    let mut fenced = String::from("```\n");
    fenced.push_str(&"a".repeat(MAX_FRAGMENT_BYTES + 1));
    fenced.push_str("\n```\n");
    let err = segment_transcript(&fenced, &policy()).expect_err("oversized fence");
    assert!(
        matches!(err, MemoryError::ContextOverLimit(_)),
        "expected ContextOverLimit, got {err:?}"
    );
}

#[test]
fn resplit_jsonl_content_override_children_get_distinct_nonoverlapping_ranges() {
    // Given one jsonl line whose *decoded* content alone exceeds
    // MAX_FRAGMENT_BYTES (m3, issue #1516): re-split into several children,
    // none of which has a byte-exact mapping back to the raw JSON-escaped
    // source line — but the partition property `compile_transcript`
    // advertises must still hold across those children.
    let big_content = "a".repeat((MAX_FRAGMENT_BYTES * 2) + 100);
    let line = serde_json::json!({"role": "user", "content": big_content}).to_string();
    let mut text = line;
    text.push('\n');

    let mut jsonl_policy = policy();
    jsonl_policy.format = SegmentFormat::Jsonl;

    let outcome = segment_transcript(&text, &jsonl_policy).expect("segments");

    // More than one child was produced — the whole point of the test
    assert!(outcome.segments.len() > 1, "{outcome:?}");

    // Every child stays under the fragment cap
    for segment in &outcome.segments {
        assert!(
            segment.fragment.content.len() <= MAX_FRAGMENT_BYTES,
            "child of {} bytes exceeds the cap",
            segment.fragment.content.len()
        );
    }

    // No two children overlap, and together they partition the whole line
    // exactly — no gaps, no duplicated ranges.
    let mut ranges: Vec<(usize, usize)> = outcome
        .segments
        .iter()
        .map(|s| (s.byte_start, s.byte_end))
        .collect();
    ranges.sort_unstable();
    let mut cursor = 0_usize;
    for (start, end) in ranges {
        assert_eq!(start, cursor, "gap or overlap at byte {cursor}");
        assert!(start <= end, "inverted range {start}..{end}");
        cursor = end;
    }
    assert_eq!(cursor, text.len(), "children must partition the whole line");
}

#[test]
fn code_fence_becomes_atomic_code_segment() {
    // Given a turn with a fenced code block
    let text = "User: run this\n```rust\nfn main() {}\n```\nUser: thanks\n";

    // When segmenting
    let outcome = segment_transcript(text, &policy()).expect("segments");

    // Then the fence is its own segment, tagged kind = code
    let code_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Code)
        .collect();
    assert_eq!(code_segments.len(), 1);
    assert_eq!(code_segments[0].fragment.kind.as_deref(), Some("code"));
    assert!(code_segments[0].fragment.content.contains("fn main()"));
}

#[test]
fn log_run_with_volatile_prefixes_becomes_log_segment() {
    // Given 8 consecutive lines with distinct ISO timestamps (volatile
    // prefix), all otherwise identical
    let mut text = String::from("User: what happened?\n");
    for i in 0..8 {
        writeln!(text, "2026-07-18T10:00:0{i}Z request handled")
            .expect("write to String never fails");
    }

    // When segmenting
    let outcome = segment_transcript(&text, &policy()).expect("segments");

    // Then the run becomes one log segment
    let log_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Log)
        .collect();
    assert_eq!(log_segments.len(), 1, "{outcome:?}");
    assert_eq!(log_segments[0].fragment.kind.as_deref(), Some("log"));
}

#[test]
fn repeated_lines_run_becomes_log_segment() {
    // Given 8 consecutive identical lines with no timestamp at all
    let mut text = String::from("User: watch this\n");
    for _ in 0..8 {
        text.push_str("connection retry failed\n");
    }

    // When segmenting
    let outcome = segment_transcript(&text, &policy()).expect("segments");

    // Then the repeated-line run still becomes one log segment
    let log_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Log)
        .collect();
    assert_eq!(log_segments.len(), 1, "{outcome:?}");
}

#[test]
fn system_turn_gets_cache_metadata() {
    // Given a transcript opening with a system turn
    let text = "System: be terse.\nUser: hello\n";

    // When segmenting with cache_system_turn on (the default)
    let outcome = segment_transcript(text, &policy()).expect("segments");

    // Then the system turn's segment(s) carry metadata.cache = true, and no
    // other turn does
    let system_segment = outcome
        .segments
        .iter()
        .find(|s| s.turn == 0)
        .expect("first turn segment");
    let metadata = system_segment.fragment.metadata.as_ref().expect("metadata");
    assert_eq!(metadata.get("cache"), Some(&Value::Bool(true)));

    let other_segment = outcome
        .segments
        .iter()
        .find(|s| s.turn == 1)
        .expect("second turn segment");
    let other_metadata = other_segment.fragment.metadata.as_ref().expect("metadata");
    assert_eq!(other_metadata.get("cache"), None);
}

#[test]
fn tiny_segments_merge_within_turn_same_kind() {
    // Given a single turn with two tiny, byte-adjacent fenced code blocks
    // (no plain text between them) — two distinct `code` pieces of the same
    // turn, each well under min_segment_bytes
    let text = "User: two blocks\n```\na\n```\n```\nb\n```\n";

    // When segmenting with a generous min_segment_bytes
    let mut merge_policy = policy();
    merge_policy.min_segment_bytes = 4096;
    let outcome = segment_transcript(text, &merge_policy).expect("segments");

    // Then the two adjacent code segments collapse into one, and the
    // outcome reports the merge — the preceding body segment (a different
    // kind) stays separate
    let code_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Code)
        .collect();
    assert_eq!(code_segments.len(), 1, "{outcome:?}");
    assert!(code_segments[0].fragment.content.contains('a'));
    assert!(code_segments[0].fragment.content.contains('b'));
    assert!(outcome.merged_segments > 0);
}

#[test]
fn segmentation_twice_is_byte_identical() {
    // Given a mixed transcript (markers, a fence, a log run)
    let mut text =
        String::from("System: be terse\nUser: run it\n```rust\nfn f() {}\n```\nAssistant: watch\n");
    for i in 0..8 {
        writeln!(text, "2026-07-18T10:00:0{i}Z tick").expect("write to String never fails");
    }

    // When segmenting it twice
    let first = segment_transcript(&text, &policy()).expect("first segmentation");
    let second = segment_transcript(&text, &policy()).expect("second segmentation");

    // Then every field of every segment matches exactly
    assert_eq!(first.segments.len(), second.segments.len());
    for (a, b) in first.segments.iter().zip(second.segments.iter()) {
        assert_eq!(a.turn, b.turn);
        assert_eq!(a.role, b.role);
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.byte_start, b.byte_start);
        assert_eq!(a.byte_end, b.byte_end);
        assert_eq!(a.fragment.content, b.fragment.content);
    }
    assert_eq!(first.format_detected, second.format_detected);
    assert_eq!(first.merged_segments, second.merged_segments);
}

#[test]
fn segment_ranges_cover_transcript_exactly() {
    // Given a mixed transcript (markers, a fence, a log run, tiny bits)
    let mut text =
        String::from("System: be terse\nUser: run it\n```rust\nfn f() {}\n```\nAssistant: watch\n");
    for i in 0..8 {
        writeln!(text, "2026-07-18T10:00:0{i}Z tick").expect("write to String never fails");
    }

    // When segmenting
    let outcome = segment_transcript(&text, &policy()).expect("segments");

    // Then the segments' byte ranges, sorted, partition [0, text.len())
    // exactly — no gaps, no overlaps
    let mut ranges: Vec<(usize, usize)> = outcome
        .segments
        .iter()
        .map(|s| (s.byte_start, s.byte_end))
        .collect();
    ranges.sort_unstable();
    let mut cursor = 0_usize;
    for (start, end) in ranges {
        assert_eq!(start, cursor, "gap or overlap at byte {cursor}");
        cursor = end;
    }
    assert_eq!(cursor, text.len());
}

#[test]
fn over_max_fragments_errors_with_actionable_message() {
    // Given a transcript whose plain markers alone produce more turns (and
    // thus segments) than MAX_FRAGMENTS allows, even with the default merge
    // threshold too small to fold them together across different turns
    let mut text = String::new();
    for i in 0..=MAX_FRAGMENTS {
        writeln!(text, "User: turn {i}").expect("write to String never fails");
    }
    let mut tight_policy = policy();
    tight_policy.min_segment_bytes = 0; // never merge — every turn stays its own segment

    // When segmenting
    let err = segment_transcript(&text, &tight_policy).expect_err("over the fragment cap");

    // Then it is an actionable error naming the remedy
    match err {
        MemoryError::ContextOverLimit(msg) => {
            assert!(msg.contains("min_segment_bytes"), "{msg}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

#[test]
fn oversized_fence_errors() {
    // Given a single fenced code block bigger than MAX_FRAGMENT_BYTES
    let mut text = String::from("```\n");
    text.push_str(&"a".repeat(MAX_FRAGMENT_BYTES + 1));
    text.push_str("\n```\n");

    // When segmenting
    let err = segment_transcript(&text, &policy()).expect_err("oversized fence");

    // Then it is a hard error, never a silent truncation
    match err {
        MemoryError::ContextOverLimit(msg) => {
            assert!(msg.contains("fenced"), "{msg}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

#[test]
fn transcript_over_cap_rejected() {
    use crate::limits::MAX_TRANSCRIPT_BYTES;

    // Given a transcript bigger than MAX_TRANSCRIPT_BYTES
    let text = "a".repeat(MAX_TRANSCRIPT_BYTES + 1);

    // When segmenting
    let err = segment_transcript(&text, &policy()).expect_err("over the transcript cap");

    // Then it is rejected before any segmentation work
    match err {
        MemoryError::ContextOverLimit(msg) => {
            assert!(msg.contains(&MAX_TRANSCRIPT_BYTES.to_string()), "{msg}");
        }
        other => panic!("expected ContextOverLimit, got {other:?}"),
    }
}

// --- Adversarial-review fixes (post-PR #1500) -------------------------------

#[test]
fn oversized_log_run_is_resplit_not_left_over_the_cap() {
    // Given ONE contiguous log run whose lines are individually tiny but
    // together exceed MAX_FRAGMENT_BYTES (B1 repro: ~1 MiB+ of timestamped
    // lines) — reject_oversized_fences only ever looked at `code` pieces
    // and resplit_oversized_bodies only ever looked at `body` pieces, so a
    // `log` piece this large used to sail through unresplit and unrejected,
    // only to blow up later at compile() time with a confusing "a fragment
    // of N bytes" error the caller never directly supplied.
    let mut text = String::from("User: watch this\n");
    let line_count = (MAX_FRAGMENT_BYTES / 30) + 100;
    for i in 0..line_count {
        writeln!(text, "2026-07-18T10:00:00Z tick {i}").expect("write to String never fails");
    }
    assert!(
        text.len() > MAX_FRAGMENT_BYTES,
        "test setup must exceed the cap"
    );

    // When segmenting
    let outcome = segment_transcript(&text, &policy()).expect("must segment, never error");

    // Then the run is resplit into several `log` segments, none over the
    // cap, and the byte ranges still partition the transcript exactly
    let log_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Log)
        .collect();
    assert!(log_segments.len() > 1, "{outcome:?}");
    for segment in &log_segments {
        assert!(
            segment.fragment.content.len() <= MAX_FRAGMENT_BYTES,
            "log segment of {} bytes exceeds the cap of {MAX_FRAGMENT_BYTES} bytes",
            segment.fragment.content.len()
        );
    }
    let mut ranges: Vec<(usize, usize)> = outcome
        .segments
        .iter()
        .map(|s| (s.byte_start, s.byte_end))
        .collect();
    ranges.sort_unstable();
    let mut cursor = 0_usize;
    for (start, end) in ranges {
        assert_eq!(start, cursor, "gap or overlap at byte {cursor}");
        cursor = end;
    }
    assert_eq!(cursor, text.len());
}

#[test]
fn merge_never_recombines_a_resplit_body_over_the_fragment_cap() {
    // Given a body whose paragraph split leaves one chunk near the cap and
    // a tiny trailing remainder (M1 repro: ~1 MiB body + a tail under
    // min_segment_bytes) — merge_tiny used to merge purely on "one side is
    // small", with no check that the COMBINED size stays under the cap.
    let big_paragraph = "a".repeat(MAX_FRAGMENT_BYTES - 5);
    let text = format!("User: {big_paragraph}\n\ntiny tail\n");

    // When segmenting with the default (small) min_segment_bytes, which
    // would otherwise want to merge the tiny tail into its same-turn,
    // same-kind neighbor
    let outcome = segment_transcript(&text, &policy()).expect("segments");

    // Then no resulting body segment exceeds the cap
    let body_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Body)
        .collect();
    assert!(body_segments.len() >= 2, "{outcome:?}");
    for segment in &body_segments {
        assert!(
            segment.fragment.content.len() <= MAX_FRAGMENT_BYTES,
            "body segment of {} bytes exceeds the cap of {MAX_FRAGMENT_BYTES} bytes",
            segment.fragment.content.len()
        );
    }
}

#[test]
fn merge_never_recombines_adjacent_fences_over_the_fragment_cap() {
    // Given two adjacent code fences whose combined size exceeds
    // MAX_FRAGMENT_BYTES even though each one alone is under the cap (M1's
    // second repro shape)
    let near_cap_body = "b".repeat(MAX_FRAGMENT_BYTES - 14);
    let text = format!("User: two blocks\n```\na\n```\n```\n{near_cap_body}\n```\n");

    // When segmenting with the default (small) min_segment_bytes
    let outcome = segment_transcript(&text, &policy()).expect("segments");

    // Then the two fences never merge into one oversized fragment
    let code_segments: Vec<_> = outcome
        .segments
        .iter()
        .filter(|s| s.kind == SegmentKind::Code)
        .collect();
    assert_eq!(code_segments.len(), 2, "{outcome:?}");
    for segment in &code_segments {
        assert!(
            segment.fragment.content.len() <= MAX_FRAGMENT_BYTES,
            "code segment of {} bytes exceeds the cap of {MAX_FRAGMENT_BYTES} bytes",
            segment.fragment.content.len()
        );
    }
}

#[test]
fn jsonl_tolerates_a_blank_line_between_turns() {
    // Given a JSONL transcript with a blank line between two real turns —
    // jsonl_pieces used to fail to parse the blank line as `{role,
    // content}`, and detect_and_segment's Auto branch swallowed that
    // error, silently falling back to a single roleless plain turn for an
    // otherwise perfectly valid JSONL transcript.
    let text = "{\"role\":\"system\",\"content\":\"be terse\"}\n\n{\"role\":\"user\",\"content\":\"hi\"}\n";

    // When segmenting with auto-detection (the default)
    let outcome = segment_transcript(text, &policy()).expect("segments");

    // Then it is still recognized as jsonl, with two correctly-numbered
    // turns and roles taken straight from the JSON
    assert_eq!(outcome.format_detected, SegmentFormat::Jsonl, "{outcome:?}");
    assert_eq!(outcome.segments.len(), 2, "{outcome:?}");
    assert_eq!(outcome.segments[0].turn, 0);
    assert_eq!(outcome.segments[0].role, Some("system".to_owned()));
    assert_eq!(outcome.segments[1].turn, 1);
    assert_eq!(outcome.segments[1].role, Some("user".to_owned()));

    // And the byte ranges still partition the transcript exactly — the
    // blank line's bytes must land somewhere, never vanish
    let mut ranges: Vec<(usize, usize)> = outcome
        .segments
        .iter()
        .map(|s| (s.byte_start, s.byte_end))
        .collect();
    ranges.sort_unstable();
    let mut cursor = 0_usize;
    for (start, end) in ranges {
        assert_eq!(start, cursor, "gap or overlap at byte {cursor}");
        cursor = end;
    }
    assert_eq!(cursor, text.len());
}
