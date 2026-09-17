//! Byte-level tests for the payload snapshot parser, through
//! `parse_payload_snapshot`: the entry the `fuzz_snapshot_parser` target calls.

use super::parse_payload_snapshot;
use super::snapshot::create_snapshot_file;
use rustc_hash::FxHashMap;
use std::io::ErrorKind;

/// The input that crashed the `fuzz_snapshot_parser` target (#2298), minimised
/// to 25 bytes: magic `VSNP`, version 1, a WAL position, then an entry count
/// of `0xafb1_ac10_ff79_ffff`, whose byte size (`count * 16`) does not fit in a `usize`.
/// The target's old copy of the parser overflowed on it; this parser rejects it.
const ENTRY_COUNT_OVERFLOW_2298: &[u8] =
    b"VSNP\x01||||||||\xff\xffy\xff\x10\xac\xb1\xaf\x01\xbe\xad\xde";

#[test]
fn test_parse_payload_snapshot_rejects_an_entry_count_whose_size_overflows() {
    let err = parse_payload_snapshot(ENTRY_COUNT_OVERFLOW_2298)
        .expect_err("a snapshot claiming more entries than it holds must be rejected");
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "Entry count exceeds data size");
}

/// The positive control: without it, an entry that rejected everything would
/// pass the test above.
#[test]
fn test_parse_payload_snapshot_accepts_a_snapshot_the_writer_wrote() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let index: FxHashMap<u64, u64> = [(1, 0), (2, 64)].into_iter().collect();
    create_snapshot_file(dir.path(), &index, 128).expect("write snapshot");
    let bytes = std::fs::read(dir.path().join("payloads.snapshot")).expect("read snapshot");
    parse_payload_snapshot(&bytes).expect("a snapshot the writer wrote must parse");
}
