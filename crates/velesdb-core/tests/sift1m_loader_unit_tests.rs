//! Unit tests for the SIFT1M loader helpers.
//!
//! Runs under `cargo test --features bench-sift1m` — the same feature
//! that gates the bench binary. These tests exercise the pure helpers
//! (`filter_groundtruth`, `verify_fingerprint`) without requiring the
//! 168 MB corpus.
//!
//! The bench binary `sift1m_recall` uses `criterion_main!`, which
//! replaces the default test harness and therefore does NOT discover
//! `#[cfg(test)]` modules. Keeping these tests in `tests/` makes them
//! runnable via the standard `cargo test` flow.

#![cfg(feature = "bench-sift1m")]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

#[path = "../benches/datasets/mod.rs"]
mod datasets;

use datasets::sift1m::filter_groundtruth;

#[test]
fn filter_groundtruth_keeps_in_range_ids_and_drops_out_of_range() {
    // 3 queries, groundtruth rows mix in-range (< n_base) and out-of-range IDs.
    // n_base = 10 -> IDs 0..=9 survive, IDs 10+ are filtered out.
    // n_query = 2 -> only the first two rows survive truncation.
    let gt = vec![
        vec![0, 5, 12, 99, 3],    // survives: [0, 5, 3]
        vec![42, 100, 1, 7, 500], // survives: [1, 7]
        vec![2, 4],               // truncated entirely — n_query = 2
    ];
    let out = filter_groundtruth(gt, 10, 2);
    assert_eq!(
        out.len(),
        2,
        "groundtruth must be truncated to n_query rows"
    );
    assert_eq!(out[0], vec![0, 5, 3]);
    assert_eq!(out[1], vec![1, 7]);
}

#[test]
fn filter_groundtruth_handles_empty_result_when_all_ids_out_of_range() {
    let gt = vec![vec![1000, 2000, 3000]];
    let out = filter_groundtruth(gt, 10, 1);
    assert_eq!(out.len(), 1);
    assert!(
        out[0].is_empty(),
        "empty row expected when all IDs exceed n_base"
    );
}

#[test]
fn filter_groundtruth_saturates_on_huge_n_base_without_overflow() {
    // When n_base > u32::MAX, the saturating conversion clamps to u32::MAX,
    // so IDs strictly less than u32::MAX are kept and u32::MAX itself is dropped.
    let gt = vec![vec![0u32, u32::MAX, 42]];
    let out = filter_groundtruth(gt, usize::MAX, 1);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0],
        vec![0, 42],
        "u32::MAX is NOT strictly less than u32::MAX, only [0, 42] survive"
    );
}

#[test]
fn filter_groundtruth_preserves_order_within_rows() {
    // Filtering must not reorder in-range IDs (downstream recall relies on
    // sequential layout for the top-k prefix).
    let gt = vec![vec![8, 20, 3, 42, 1, 99]];
    let out = filter_groundtruth(gt, 10, 1);
    assert_eq!(out[0], vec![8, 3, 1]);
}
