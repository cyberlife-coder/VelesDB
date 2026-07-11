//! Property-based tests for WAL retention safety.
//!
//! Feature: core-control-plane-boundary, Property 9.
//! These tests live in their own module so Property 9 (retention) coverage
//! stays clearly delineated from the Property 8 (monotonicity/contiguity)
//! tests and the legacy-compat tests that target the same WAL cursor seam.
//!
//! # Contract under test
//!
//! Retention is **hold-all-or-reclaim-all**: full truncation of the WAL is
//! only allowed when the minimum registered low-watermark `W` has already
//! reached the append tail (`watermark_allows_full_truncation`); otherwise the
//! reclaim is held and nothing is discarded — a permitted subset of "records
//! below `W`". Combined with the concrete on-disk truncation performed by
//! compaction (`set_len(0)`), this test asserts the safety invariant directly:
//! after the truncation decision, every record at position `>= W` remains
//! readable through [`LogWalCursor`], and only records `< W` may be reclaimed.

use super::log_payload::LogPayloadStorage;
use super::traits::PayloadStorage;
use super::wal_cursor::{
    watermark_allows_full_truncation, WalCursor, WalPosition, WalRecord, WalWatermarkRegistry,
};
use super::wal_cursor_reader::LogWalCursor;
use proptest::prelude::*;
use std::fs::OpenOptions;
use std::path::Path;

/// A synthetic WAL operation used to build a real `payloads.log` on disk.
#[derive(Debug, Clone)]
enum WalOp {
    /// Store (upsert) a payload of `payload_len` bytes under `id`.
    Store { id: u64, payload_len: usize },
    /// Delete `id` (writes a tombstone frame only if `id` is currently stored).
    Delete { id: u64 },
}

/// Generates a single synthetic WAL operation over a small id space so stores
/// and deletes interleave and actually hit previously-stored ids.
fn wal_op_strategy() -> impl Strategy<Value = WalOp> {
    prop_oneof![
        (0u64..8, 0usize..24).prop_map(|(id, payload_len)| WalOp::Store { id, payload_len }),
        (0u64..8).prop_map(|id| WalOp::Delete { id }),
    ]
}

/// Applies the generated ops to a real `LogPayloadStorage`, writing the actual
/// marker/CRC WAL framing to `payloads.log`, then flushes for durability.
fn build_wal(dir: &Path, ops: &[WalOp]) {
    let mut storage = LogPayloadStorage::new(dir).expect("open log payload storage in temp dir");
    for op in ops {
        match op {
            WalOp::Store { id, payload_len } => {
                let payload = serde_json::json!({ "v": "x".repeat(*payload_len) });
                storage.store(*id, &payload).expect("store payload");
            }
            WalOp::Delete { id } => {
                storage.delete(*id).expect("delete payload");
            }
        }
    }
    storage.flush().expect("flush wal");
    // Drop the writer so a fresh handle can truncate the file cleanly below.
    drop(storage);
}

/// Reads every record from `START` in bounded batches and returns the full
/// ordered record list.
fn drain_from_start(cursor: &LogWalCursor) -> Vec<WalRecord> {
    let mut all = Vec::new();
    let mut pos = WalPosition::START;
    loop {
        let batch = cursor.read_from(pos, 4).expect("read_from");
        if batch.is_empty() {
            break;
        }
        pos = batch[batch.len() - 1].next;
        all.extend(batch);
    }
    all
}

/// Performs the concrete on-disk truncation compaction would perform when the
/// retention contract allows full reclaim: truncates `payloads.log` to empty,
/// exactly matching `reclaim_wal_if_unheld`'s `set_len(0)`.
fn reclaim_full(dir: &Path) {
    let file = OpenOptions::new()
        .write(true)
        .open(dir.join("payloads.log"))
        .expect("open wal for truncation");
    file.set_len(0).expect("truncate wal to empty");
}

// Feature: core-control-plane-boundary, Property 9: WAL retention never
// discards a needed position — with min registered low-watermark `W`, after a
// truncation decision every record at position `>= W` remains readable and
// only records `< W` may be reclaimed; with no registered consumer, truncation
// matches the pre-cursor full-truncation baseline.
// **Validates: Requirements 6.3**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// A single registered consumer at low-watermark `W` never loses a needed
    /// position: the hold-all-or-reclaim-all decision keeps every record at
    /// `>= W` readable and only ever reclaims records `< W`.
    #[test]
    fn prop_retention_never_discards_needed_position(
        ops in prop::collection::vec(wal_op_strategy(), 1..30),
        // `W` selector: an arbitrary offset spanning below, at, and beyond the
        // tail so both the "held" (W < tail) and "reclaimable" (W >= tail)
        // branches of the contract are exercised.
        w_raw in 0u64..512,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        build_wal(dir.path(), &ops);

        let original = {
            let cursor = LogWalCursor::new(dir.path());
            drain_from_start(&cursor)
        };
        prop_assume!(!original.is_empty());

        let tail = original.last().map_or(0, |r| r.next.offset());
        // Constrain the chosen watermark to the meaningful range [0, tail + 8]
        // so we cover positions inside the log and just past its tail.
        let w = w_raw % (tail + 8);
        let watermark = WalPosition::new(w);

        // Register one consumer and advance it to `W` (monotonic from START).
        let registry = WalWatermarkRegistry::new();
        let consumer = registry.register();
        registry.advance(consumer, watermark);

        let min = registry.min_watermark();
        prop_assert_eq!(min, Some(watermark));

        // The contract decision: full reclaim only when W has reached the tail.
        let allows_full = watermark_allows_full_truncation(min, tail);
        prop_assert_eq!(allows_full, w >= tail);

        // The records a consumer at `W` still needs (positions `>= W`).
        let needed: Vec<&WalRecord> = original
            .iter()
            .filter(|r| r.position.offset() >= w)
            .collect();

        // Apply the truncation decision, then re-read what survives.
        if allows_full {
            reclaim_full(dir.path());
        }
        let after = {
            let cursor = LogWalCursor::new(dir.path());
            drain_from_start(&cursor)
        };

        // Safety invariant: every needed record (position >= W) is still
        // readable after the truncation decision.
        for record in &needed {
            prop_assert!(
                after.iter().any(|a| a == *record),
                "record at position {} (>= W {}) was discarded",
                record.position.offset(),
                w
            );
        }

        // Only records strictly below `W` may be reclaimed: anything present
        // before but absent after must have position < W.
        for record in &original {
            let survived = after.iter().any(|a| a == record);
            if !survived {
                prop_assert!(
                    record.position.offset() < w,
                    "reclaimed record at position {} was not below W {}",
                    record.position.offset(),
                    w
                );
            }
        }
    }

    /// With no registered consumer, the contract permits full reclaim and the
    /// truncation matches the pre-cursor baseline: the WAL is truncated to
    /// empty exactly as it was before this API existed.
    #[test]
    fn prop_no_consumer_matches_pre_cursor_baseline(
        ops in prop::collection::vec(wal_op_strategy(), 1..30),
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        build_wal(dir.path(), &ops);

        let original = {
            let cursor = LogWalCursor::new(dir.path());
            drain_from_start(&cursor)
        };
        prop_assume!(!original.is_empty());
        let tail = original.last().map_or(0, |r| r.next.offset());

        // No consumer registered → no retention hold.
        let registry = WalWatermarkRegistry::new();
        prop_assert_eq!(registry.min_watermark(), None);
        prop_assert!(watermark_allows_full_truncation(None, tail));

        // Baseline reclaim (full truncation), matching today's behavior.
        reclaim_full(dir.path());
        let after = {
            let cursor = LogWalCursor::new(dir.path());
            drain_from_start(&cursor)
        };
        prop_assert!(after.is_empty());
        prop_assert_eq!(LogWalCursor::new(dir.path()).tail_position(), WalPosition::START);
    }

    /// A consumer whose low-watermark sits strictly below the tail holds the
    /// entire WAL: the decision is "held", nothing is reclaimed, and every
    /// record — including those at `>= W` — remains readable.
    #[test]
    fn prop_consumer_below_tail_holds_all(
        ops in prop::collection::vec(wal_op_strategy(), 1..30),
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        build_wal(dir.path(), &ops);

        let original = {
            let cursor = LogWalCursor::new(dir.path());
            drain_from_start(&cursor)
        };
        prop_assume!(!original.is_empty());
        let tail = original.last().map_or(0, |r| r.next.offset());

        // Advance the consumer to a position strictly below the tail.
        let registry = WalWatermarkRegistry::new();
        let consumer = registry.register();
        let w = tail.saturating_sub(1);
        registry.advance(consumer, WalPosition::new(w));

        // Held: full truncation is not allowed while W < tail.
        prop_assert!(!watermark_allows_full_truncation(registry.min_watermark(), tail));

        // Decision is "hold" → nothing is truncated → every record survives.
        let after = {
            let cursor = LogWalCursor::new(dir.path());
            drain_from_start(&cursor)
        };
        prop_assert_eq!(after.len(), original.len());
        prop_assert_eq!(&after[..], &original[..]);
    }
}
