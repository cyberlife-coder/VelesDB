//! Bench-only counter of sparse scoring work, in posting inspections.
//!
//! Compiled only with the `internal-bench` feature: release and default test
//! builds carry zero instrumentation. It exists to answer #2177's remaining
//! question — whether `maxscore_search` earns its keep above
//! `SMALL_CORPUS_LINEAR_THRESHOLD` — with a number that means the same thing
//! on every machine.
//!
//! # The unit
//!
//! One *inspection* is one posting entry looked at. It is counted at the
//! points where the two strategies genuinely differ in work:
//!
//! - `linear_scan_dense` / `linear_scan_hashmap`: one per posting accumulated,
//!   which is the whole of their scoring cost;
//! - `find_min_essential_doc_id`: one per essential cursor peeked, per step;
//! - `score_document`: one per essential term matched at the cursor, plus the
//!   probes a non-essential term's binary search performs.
//!
//! Binary-search probes are counted as `ilog2(len) + 1`, the number
//! `binary_search` performs on a list of that length, rather than by
//! reimplementing the search to count each step. That keeps the standard
//! library's search on the hot path and the count exact for the power-of-two
//! case and within one probe otherwise — stated here so nobody reads the
//! figure as a measured probe trace.
//!
//! What is deliberately NOT counted: `get_all_postings`, which both strategies
//! call identically, and the top-k heap, which both pay per surviving
//! candidate. #2177 measured the posting clones as common to both; counting
//! them would inflate both sides equally and hide the ratio that matters.
//!
//! # Corollary
//!
//! With `internal-bench` enabled, a relaxed atomic increment sits inside the
//! scoring loops — wall-clock measured under this feature is NOT a benchmark
//! of the real engine. Count with this feature; time without it, exactly as
//! `index::hnsw::eval_count` says for the dense side.

use std::sync::atomic::{AtomicU64, Ordering};

/// Total posting inspections since the last reset.
static SCORING_OPS: AtomicU64 = AtomicU64::new(0);

/// Records `n` posting inspections.
#[inline]
pub(crate) fn record_ops(n: u64) {
    SCORING_OPS.fetch_add(n, Ordering::Relaxed);
}

/// Records the probes `binary_search` performs over `len` entries.
///
/// `ilog2(len) + 1` for a non-empty list, and nothing for an empty one, which
/// `binary_search` answers without a probe.
#[inline]
pub(crate) fn record_binary_search(len: usize) {
    if let Ok(len) = u64::try_from(len) {
        if len > 0 {
            record_ops(u64::from(len.ilog2()) + 1);
        }
    }
}

/// Returns the posting inspections counted since the last reset.
#[must_use]
pub(crate) fn scoring_ops() -> u64 {
    SCORING_OPS.load(Ordering::Relaxed)
}

/// Resets the inspection counter to zero.
pub(crate) fn reset_scoring_ops() {
    SCORING_OPS.store(0, Ordering::Relaxed);
}
