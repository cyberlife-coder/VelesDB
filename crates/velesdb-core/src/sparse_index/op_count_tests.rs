//! The counter's arithmetic, pinned exactly.
//!
//! `cargo mutants` found seven survivors on this PR and six were here: making
//! `record_binary_search` a no-op, turning its `len > 0` into `==`, `<` or
//! `>=`, and its `+ 1` into `* 1` all left every test green. That matters more
//! than an ordinary coverage gap: this counter IS #2177's measurement. A
//! no-op version turns the ratio it reports above a million documents into
//! nothing at all, silently, and the retirement decision would rest on a
//! number no test defends. The figures themselves are deliberately absent
//! from this comment: `tests/sparse_strategy_crossover.rs` produces them at
//! run time, and a copy sitting in a doc comment is exactly the stale
//! duplicate that cost #2344 four review findings.
//!
//! So the values below are asserted exactly rather than as "greater than
//! zero". An exact count is also what the harness promises — it calls itself
//! bit-for-bit reproducible across machines — and a promise nothing checks is
//! the failure mode this PR spent its review rounds on.
//!
//! `#[serial]` because the counter is a process-global atomic: run in parallel
//! these cases reset each other's baseline and the exact values become
//! nonsense. Measured, not feared — `cargo test --lib op_count` fails two of
//! four in parallel and passes all four at `--test-threads=1`. CI imposes
//! single-threading, the pre-commit hook does not, which is the same gap that
//! caught #2344's parking tests.

use super::op_count::{record_binary_search, record_ops, reset_scoring_ops, scoring_ops};
use serial_test::serial;

/// Counts are process-global, so each case starts from a known zero.
fn counted(f: impl FnOnce()) -> u64 {
    reset_scoring_ops();
    f();
    scoring_ops()
}

#[test]
#[serial]
fn a_binary_search_over_an_empty_list_probes_nothing() {
    // `binary_search` answers an empty slice without a probe. This is also
    // what keeps `len > 0` from becoming `>=`: `ilog2(0)` panics.
    assert_eq!(counted(|| record_binary_search(0)), 0);
}

#[test]
#[serial]
fn a_binary_search_costs_ilog2_plus_one_probes() {
    // Exact, not approximate: `+ 1` mutated to `* 1` changes 4 into 3 here.
    assert_eq!(counted(|| record_binary_search(1)), 1, "ilog2(1) + 1");
    assert_eq!(counted(|| record_binary_search(2)), 2, "ilog2(2) + 1");
    assert_eq!(counted(|| record_binary_search(8)), 4, "ilog2(8) + 1");
    assert_eq!(counted(|| record_binary_search(9)), 4, "ilog2 floors");
    assert_eq!(
        counted(|| record_binary_search(1024)),
        11,
        "ilog2(1024) + 1"
    );
}

#[test]
#[serial]
fn inspections_accumulate_until_reset() {
    let total = counted(|| {
        record_ops(3);
        record_ops(4);
        record_binary_search(8);
    });
    assert_eq!(total, 11, "3 + 4 + 4");
    reset_scoring_ops();
    assert_eq!(scoring_ops(), 0, "a reset that leaves a residue carries it");
}
