//! Property-based tests for WAL cursor monotonicity and contiguity.
//!
//! Feature: core-control-plane-boundary, Property 8.
//! These tests live in their own module so Property 8 coverage stays clearly
//! delineated from the retention (Property 9) and legacy-compat tests that
//! target the same WAL cursor seam.

use super::log_payload::LogPayloadStorage;
use super::traits::PayloadStorage;
use super::wal_cursor::{WalCursor, WalPosition, WalRecord};
use super::wal_cursor_reader::LogWalCursor;
use proptest::prelude::*;

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
fn build_wal(dir: &std::path::Path, ops: &[WalOp]) -> LogPayloadStorage {
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
    storage
}

/// Reads every record from `START` in bounded batches, exercising the cursor's
/// resume-from-`next` contract, and returns the full ordered record list.
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

// Feature: core-control-plane-boundary, Property 8: WAL cursor positions are
// monotonic and contiguous — reading from `START` yields strictly increasing
// positions; the `next` of record i equals the `position` of record i+1; and
// `read_from(p, max)` returns exactly the suffix at positions `>= p`, up to
// `max` records.
// **Validates: Requirements 6.2**
proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Full contiguity + monotonicity walk over a synthetic on-disk WAL.
    #[test]
    fn prop_positions_monotonic_and_contiguous(ops in prop::collection::vec(wal_op_strategy(), 0..30)) {
        let dir = tempfile::tempdir().expect("tempdir");
        let _storage = build_wal(dir.path(), &ops);
        let cursor = LogWalCursor::new(dir.path());

        let all = drain_from_start(&cursor);

        // The first record (if any) begins at START.
        if let Some(first) = all.first() {
            prop_assert_eq!(first.position, WalPosition::START);
        }

        // Strictly increasing positions + exact contiguity between neighbours.
        for pair in all.windows(2) {
            prop_assert!(pair[1].position.offset() > pair[0].position.offset());
            prop_assert_eq!(pair[0].next, pair[1].position);
        }

        // Each record's `next` is strictly past its own start (records are
        // non-empty frames), and equals start + framed byte length.
        for record in &all {
            prop_assert!(record.next.offset() > record.position.offset());
            let framed_len = record.next.offset() - record.position.offset();
            prop_assert_eq!(framed_len, record.bytes.len() as u64);
        }

        // The tail is exactly the `next` of the last record (or START when empty),
        // and reading from the tail yields nothing.
        let expected_tail = all.last().map_or(WalPosition::START, |r| r.next);
        prop_assert_eq!(cursor.tail_position(), expected_tail);
        prop_assert!(cursor.read_from(cursor.tail_position(), 16).expect("read tail").is_empty());
    }

    /// `read_from(p, max)` returns exactly the suffix at positions `>= p`,
    /// truncated to `max`, for every record boundary `p`.
    #[test]
    fn prop_read_from_returns_exact_suffix(
        ops in prop::collection::vec(wal_op_strategy(), 1..30),
        max in 1usize..40,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let _storage = build_wal(dir.path(), &ops);
        let cursor = LogWalCursor::new(dir.path());

        let all = drain_from_start(&cursor);
        prop_assume!(!all.is_empty());

        for (i, start) in all.iter().enumerate() {
            let got = cursor.read_from(start.position, max).expect("read_from suffix");
            let expected_len = (all.len() - i).min(max);
            prop_assert_eq!(got.len(), expected_len);
            for (offset, record) in got.iter().enumerate() {
                prop_assert_eq!(record, &all[i + offset]);
            }
        }

        // A very large `max` returns the entire suffix from any boundary.
        let mid = all.len() / 2;
        let full_suffix = cursor.read_from(all[mid].position, usize::MAX).expect("read full suffix");
        prop_assert_eq!(full_suffix.len(), all.len() - mid);
        prop_assert_eq!(&full_suffix[..], &all[mid..]);
    }
}
