//! Tests for the remaining-length guard the pointer-walked kernels use.
//!
//! The property that matters is not "does it count chunks" — that was never
//! wrong — but that it answers the same question the old `p.add(N) <= end`
//! guard answered, at every length, without forming the pointer that made the
//! old form UB. So the tests pin the *answer*, exhaustively over the lengths
//! where the two could disagree, and pin the loop that consumes it.
//!
//! Deliberately free of intrinsics, so this file runs under Miri
//! (`cargo +nightly miri test -p velesdb-core ptr_span`) — which the kernels
//! themselves cannot, Miri having no AVX-512 support. That is what makes this
//! the place the soundness claim can be checked rather than asserted.

use super::ptr_span::has_at_least;

/// What `p.add(count) <= end` would have said, computed on integers.
///
/// The reference for equivalence, deliberately not written with pointer
/// arithmetic: the whole point is that the pointer form cannot legally be
/// evaluated at the boundary this compares against.
fn remaining(consumed: usize, len: usize, count: usize) -> bool {
    len - consumed >= count
}

/// The guard agrees with the arithmetic answer at every length and position.
///
/// Ranges chosen to straddle both lane widths in use (8 for AVX2, 16 for
/// AVX-512) several times over, so an off-by-one at a chunk boundary shows up
/// rather than hiding between the probed points.
#[test]
fn the_guard_matches_the_remaining_element_count_everywhere() {
    for len in 0..=80usize {
        let buffer = vec![0.0f32; len];
        let start = buffer.as_ptr();
        // SAFETY: one past the last element is in bounds for `add`.
        let end = unsafe { start.add(len) };

        for consumed in 0..=len {
            // SAFETY: `consumed <= len`, so this stays inside the allocation.
            let cur = unsafe { start.add(consumed) };
            for count in [1usize, 2, 4, 8, 16, 32] {
                assert_eq!(
                    has_at_least(cur, end, count),
                    remaining(consumed, len, count),
                    "len {len}, consumed {consumed}, count {count}"
                );
            }
        }
    }
}

/// An empty range never has room, whatever is asked of it.
///
/// `Vec::as_ptr` on an empty vector returns a dangling-but-aligned pointer and
/// `start == end`. The guard must read that as "nothing remains" rather than
/// subtracting two unrelated addresses.
#[test]
fn an_empty_range_has_room_for_nothing() {
    let empty: Vec<f32> = Vec::new();
    let start = empty.as_ptr();
    for count in [1usize, 8, 16] {
        assert!(!has_at_least(start, start, count));
    }
    // Zero elements always fit, including here: the loops never ask, but a
    // guard that answered `false` would be saying something untrue.
    assert!(has_at_least(start, start, 0));
}

/// A cursor past the end reports "nothing remains" instead of wrapping.
///
/// Unreachable through the kernels' own loop structure, which is exactly why
/// it is worth pinning: `end - cur` on `usize` would wrap to a huge span and
/// wave the loop straight off the end of the buffer. The saturating form fails
/// closed, and this is the test that says so.
#[test]
fn a_cursor_past_the_end_fails_closed() {
    let buffer = [0.0f32; 32];
    let start = buffer.as_ptr();
    // SAFETY: both offsets are within the 32-element allocation.
    let (end, past) = unsafe { (start.add(16), start.add(24)) };

    assert!(!has_at_least(past, end, 1));
    assert!(!has_at_least(past, end, 16));
}

/// The consuming loop terminates and covers exactly the full chunks.
///
/// The guard is only ever read as a `while` condition, so the property the
/// kernels actually depend on is this one: the loop runs `len / count` times
/// and leaves the cursor with `len % count` elements to hand to the tail.
#[test]
fn the_loop_consumes_every_full_chunk_and_no_more() {
    for len in 0..=80usize {
        for count in [8usize, 16] {
            let buffer = vec![0.0f32; len];
            let start = buffer.as_ptr();
            // SAFETY: one past the last element is in bounds for `add`.
            let end = unsafe { start.add(len) };

            let mut cur = start;
            let mut chunks = 0usize;
            while has_at_least(cur, end, count) {
                chunks += 1;
                // SAFETY: the guard just proved `count` elements remain.
                cur = unsafe { cur.add(count) };
            }

            assert_eq!(chunks, len / count, "len {len}, count {count}");
            // SAFETY: `cur` is within the allocation; so is `end`.
            let left = unsafe { end.offset_from(cur) };
            assert_eq!(
                usize::try_from(left).expect("cursor never passes end"),
                len % count,
                "len {len}, count {count}: tail left for the scalar path"
            );
        }
    }
}
