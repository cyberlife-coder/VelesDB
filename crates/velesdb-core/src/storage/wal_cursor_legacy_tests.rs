//! Unit tests for legacy WAL compatibility through [`LogWalCursor`].
//!
//! Feature: core-control-plane-boundary, Requirement 6.4.
//! Requirement 6.4 mandates that the WAL-shippability seam records the
//! on-disk-format impact and preserves compatibility with existing stored
//! data. The [`LogWalCursor`] introduces no on-disk change, so a WAL written
//! by an older version — using the no-CRC legacy framing (markers `1`/`2`) —
//! must still read back correctly through the cursor, both on its own and
//! interleaved with the current CRC framing (markers `0xC3`/`0xC4`).
//!
//! These live in their own module so legacy-compat coverage stays clearly
//! delineated from Property 8 (`wal_cursor_property_tests`) and Property 9
//! (`wal_retention_property_tests`), which target the same cursor seam.

use super::log_payload_io::{
    compute_delete_crc, compute_store_crc, CRC_DELETE_MARKER, CRC_STORE_MARKER,
    LEGACY_DELETE_MARKER, LEGACY_STORE_MARKER,
};
use super::wal_cursor::{WalCursor, WalPosition};
use super::wal_cursor_reader::LogWalCursor;

/// Appends a legacy (no-CRC) store frame: `marker(1) | id(8) | len(4) | payload`.
fn push_legacy_store(buf: &mut Vec<u8>, id: u64, payload: &[u8]) {
    let len = u32::try_from(payload.len()).expect("test payload fits in u32");
    buf.push(LEGACY_STORE_MARKER);
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
}

/// Appends a legacy (no-CRC) delete frame: `marker(2) | id(8)`.
fn push_legacy_delete(buf: &mut Vec<u8>, id: u64) {
    buf.push(LEGACY_DELETE_MARKER);
    buf.extend_from_slice(&id.to_le_bytes());
}

/// Appends a CRC store frame: `marker(0xC3) | id(8) | len(4) | payload | crc(4)`.
fn push_crc_store(buf: &mut Vec<u8>, id: u64, payload: &[u8]) {
    let len = u32::try_from(payload.len()).expect("test payload fits in u32");
    buf.push(CRC_STORE_MARKER);
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    buf.extend_from_slice(&compute_store_crc(id, payload).to_le_bytes());
}

/// Appends a CRC delete frame: `marker(0xC4) | id(8) | crc(4)`.
fn push_crc_delete(buf: &mut Vec<u8>, id: u64) {
    buf.push(CRC_DELETE_MARKER);
    buf.extend_from_slice(&id.to_le_bytes());
    buf.extend_from_slice(&compute_delete_crc(id).to_le_bytes());
}

/// Writes raw framed bytes to `<dir>/payloads.log` — the file
/// [`LogWalCursor::new`] reads.
fn write_log(dir: &std::path::Path, bytes: &[u8]) {
    std::fs::write(dir.join("payloads.log"), bytes).expect("write payloads.log");
}

/// Drains every record from `START` in small batches (exercising resume-from
/// `next`), asserting each frame's bytes equal the expected slice of the
/// original log and that positions are contiguous.
fn assert_frames(cursor: &LogWalCursor, log: &[u8], expected_frames: &[&[u8]]) {
    let mut all = Vec::new();
    let mut pos = WalPosition::START;
    loop {
        let batch = cursor.read_from(pos, 3).expect("read_from");
        if batch.is_empty() {
            break;
        }
        pos = batch[batch.len() - 1].next;
        all.extend(batch);
    }

    assert_eq!(all.len(), expected_frames.len(), "record count");

    let mut offset = 0usize;
    for (record, expected) in all.iter().zip(expected_frames.iter()) {
        // Position sits at the running offset; the frame is contiguous.
        assert_eq!(record.position.offset(), offset as u64, "frame position");
        // Bytes are the exact framed slice from the on-disk log.
        assert_eq!(record.bytes.as_slice(), *expected, "framed bytes");
        let framed_len = record.bytes.len();
        assert_eq!(
            &log[offset..offset + framed_len],
            record.bytes.as_slice(),
            "bytes match on-disk slice"
        );
        // `next` is exactly `position + framed length`.
        assert_eq!(
            record.next.offset(),
            (offset + framed_len) as u64,
            "next cursor"
        );
        offset += framed_len;
    }

    // The tail equals the end of the last frame (the full file length), and
    // reading from it yields nothing.
    assert_eq!(
        cursor.tail_position().offset(),
        offset as u64,
        "tail at file end"
    );
    assert_eq!(offset, log.len(), "consumed the whole log");
    assert!(
        cursor
            .read_from(cursor.tail_position(), 16)
            .expect("read tail")
            .is_empty(),
        "no records past tail"
    );
}

/// A WAL written entirely in the legacy no-CRC framing reads back correctly:
/// every legacy store and delete frame is yielded, contiguous, with bytes
/// identical to the on-disk record.
#[test]
fn reads_pure_legacy_wal() {
    let dir = tempfile::tempdir().expect("tempdir");

    let mut log = Vec::new();
    let mut frames: Vec<Vec<u8>> = Vec::new();

    let mut record = |build: &dyn Fn(&mut Vec<u8>)| {
        let start = log.len();
        build(&mut log);
        frames.push(log[start..].to_vec());
    };

    record(&|b| push_legacy_store(b, 1, br#"{"v":"alpha"}"#));
    record(&|b| push_legacy_store(b, 2, b"")); // zero-length payload edge case
    record(&|b| push_legacy_delete(b, 1));
    record(&|b| push_legacy_store(b, 3, br#"{"v":"a much longer legacy payload"}"#));
    record(&|b| push_legacy_delete(b, 2));

    write_log(dir.path(), &log);

    let cursor = LogWalCursor::new(dir.path());
    let expected: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    assert_frames(&cursor, &log, &expected);
}

/// A WAL that mixes legacy no-CRC frames with current CRC frames — as would
/// exist after an older store is reopened and appended to by a newer version —
/// reads back every frame correctly regardless of framing.
#[test]
fn reads_mixed_legacy_and_crc_wal() {
    let dir = tempfile::tempdir().expect("tempdir");

    let mut log = Vec::new();
    let mut frames: Vec<Vec<u8>> = Vec::new();

    let mut record = |build: &dyn Fn(&mut Vec<u8>)| {
        let start = log.len();
        build(&mut log);
        frames.push(log[start..].to_vec());
    };

    // Older (legacy) prefix, then a newer (CRC) suffix — interleaved markers.
    record(&|b| push_legacy_store(b, 10, br#"{"legacy":true}"#));
    record(&|b| push_crc_store(b, 11, br#"{"crc":true}"#));
    record(&|b| push_legacy_delete(b, 10));
    record(&|b| push_crc_delete(b, 11));
    record(&|b| push_crc_store(b, 12, br#"{"crc":"final"}"#));
    record(&|b| push_legacy_store(b, 13, b"tail-legacy"));

    write_log(dir.path(), &log);

    let cursor = LogWalCursor::new(dir.path());
    let expected: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    assert_frames(&cursor, &log, &expected);
}
