//! Byte-level tests for the payload snapshot parser, through
//! `parse_payload_snapshot`: the entry the `fuzz_snapshot_parser` target calls.

use super::parse_payload_snapshot;
use super::snapshot::{create_snapshot_file, load_snapshot};
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

/// An unreachable snapshot is an error, not an absence (#2325).
///
/// `Path::exists` answers `false` when the metadata cannot be read at all —
/// "e.g. because of a permission error or broken symbolic links" (std 1.90).
/// `load_snapshot` turned that into `NotFound`, which
/// `LogPayloadStore::load_or_replay` reads as the cold-start case: it replays
/// the WAL with **no log line**, while every other error kind gets a
/// `warn!` first. No data is lost — the replay rebuilds the index — but an
/// operator gets no signal that the fast path was skipped, and startup pays
/// the full replay on every boot until someone notices.
///
/// The fix is `try_exists()?`, which distinguishes "not there" from "cannot
/// tell". This asserts the distinction on a directory the process cannot
/// enter.
#[cfg(unix)]
#[test]
fn an_unreachable_snapshot_is_an_error_not_a_missing_one() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("test: temp dir");
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).expect("test: create the locked dir");
    let snapshot = locked.join("payload.snapshot");
    std::fs::write(&snapshot, b"whatever").expect("test: write the snapshot");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000))
        .expect("test: chmod 000");

    // Observe first, restore second, assert last: a panic between the chmod
    // and the restore would leave an unremovable directory behind and the
    // temp dir would fail to clean up, turning a clear verdict into noise.
    let reachable = std::fs::read(&snapshot).is_ok();
    let outcome = load_snapshot(&snapshot);
    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700));

    // The premise: this process really cannot reach the file. Root ignores
    // the mode, and a test whose premise is absent proves nothing, so say so
    // instead of passing quietly.
    assert!(
        !reachable,
        "the locked directory is still readable (running as root?): this test \
         cannot observe what it exists to observe"
    );

    let err = outcome.expect_err("an unreachable snapshot must not load");
    assert_ne!(
        err.kind(),
        std::io::ErrorKind::NotFound,
        "an unreachable snapshot was reported as absent, so the caller replays \
         the WAL without a word: {err}"
    );
}
