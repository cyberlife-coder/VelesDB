//! A WAL append that fails partway through a record leaves the WAL as if the
//! record never started (#2850): no fragment for the next record to land
//! behind, nothing the replay of the next open would stop at, and nothing
//! recorded in the index or the change log for a write that changed
//! nothing.

use super::log_payload::{DurabilityMode, LogPayloadStorage};
use super::log_payload_io::{
    discard_torn_record, tear_next_wal_append, tear_wal_append_after, CRC_STORE_MARKER,
};
use super::traits::PayloadStorage;
use serde_json::json;
use tempfile::TempDir;

/// Stores 1, tears the append `tear` makes, stores 3, and checks 1 and 3
/// read back — before and after a reopen — and the torn write changed
/// nothing; `changed` is what the torn write left in the change log.
fn assert_recovers_after_torn_append(
    durability: DurabilityMode,
    tear: impl FnOnce(&mut LogPayloadStorage) -> std::io::Result<()>,
    changed: &[u64],
) {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut storage =
        LogPayloadStorage::new_with_durability(temp_dir.path(), durability).expect("open");
    storage.store(1, &json!({"a": 1})).expect("store 1");
    let before = storage.changes().revision();
    {
        let _fault = tear_next_wal_append();
        assert!(tear(&mut storage).is_err(), "precondition: the append tore");
    }
    assert_eq!(
        storage.changes().changed_since(before),
        Some(changed.to_vec()),
        "the ids the torn write changed"
    );
    storage.store(3, &json!({"c": 3})).expect("store 3");

    assert_eq!(storage.retrieve(1).expect("read 1"), Some(json!({"a": 1})));
    assert_eq!(storage.retrieve(3).expect("read 3"), Some(json!({"c": 3})));
    assert_eq!(storage.retrieve(2).expect("read 2"), None);
    drop(storage);

    let reopened = LogPayloadStorage::new(temp_dir.path()).expect("reopen");
    assert_eq!(reopened.retrieve(1).expect("read 1"), Some(json!({"a": 1})));
    assert_eq!(reopened.retrieve(3).expect("read 3"), Some(json!({"c": 3})));
    assert_eq!(reopened.retrieve(2).expect("read 2"), None);
}

#[test]
fn test_torn_store_leaves_no_fragment_in_the_wal() {
    assert_recovers_after_torn_append(
        DurabilityMode::Fsync,
        |s| s.store(2, &json!({"b": "torn"})),
        &[],
    );
}

#[test]
fn test_torn_store_batch_leaves_no_fragment_in_the_wal() {
    assert_recovers_after_torn_append(
        DurabilityMode::Fsync,
        |s| s.store_batch(&[(2, &json!({"b": "torn"}))]),
        &[],
    );
}

/// A batch torn at its second record keeps its first, which reached the WAL
/// and the index: the change log holds it, and nothing of the torn one —
/// before and after a reopen.
#[test]
fn test_a_batch_torn_midway_keeps_and_records_what_it_wrote() {
    let temp_dir = TempDir::new().expect("temp dir");
    let mut storage = LogPayloadStorage::new(temp_dir.path()).expect("open");
    let before = storage.changes().revision();
    let written = json!({"w": 1});
    {
        let _fault = tear_wal_append_after(1);
        let torn = storage.store_batch(&[(10, &written), (11, &json!({"t": "torn"}))]);
        assert!(torn.is_err(), "precondition: the second append tore");
    }
    assert_eq!(storage.changes().changed_since(before), Some(vec![10]));
    storage
        .store(12, &json!({"later": 1}))
        .expect("a later store");

    assert_eq!(
        storage.retrieve(10).expect("read 10"),
        Some(written.clone())
    );
    assert_eq!(storage.retrieve(11).expect("read 11"), None);
    drop(storage);
    let reopened = LogPayloadStorage::new(temp_dir.path()).expect("reopen");
    assert_eq!(reopened.retrieve(10).expect("read 10"), Some(written));
    assert_eq!(reopened.retrieve(11).expect("read 11"), None);
    assert_eq!(
        reopened.retrieve(12).expect("read 12"),
        Some(json!({"later": 1}))
    );
}

/// Under `DurabilityMode::None` record 1 is still buffered when record 2
/// tears: the tear flushes it first, and the truncation must keep it.
#[test]
fn test_torn_store_keeps_the_buffered_record_before_it() {
    assert_recovers_after_torn_append(
        DurabilityMode::None,
        |s| s.store(2, &json!({"b": "torn"})),
        &[],
    );
}

/// A torn tombstone is not applied: the point stays live, now and on reopen.
#[test]
fn test_torn_delete_leaves_no_fragment_in_the_wal() {
    assert_recovers_after_torn_append(DurabilityMode::Fsync, |s| s.delete(1), &[]);
}

#[test]
fn test_torn_delete_batch_leaves_no_fragment_in_the_wal() {
    assert_recovers_after_torn_append(DurabilityMode::Fsync, |s| s.delete_batch(&[1]), &[]);
}

/// An append that fails while flushing earlier, acknowledged records leaves
/// part of them in the file and the rest buffered, and no fragment of the
/// new record. The truncation must find the file already at its expected
/// length and cut nothing: those bytes land on the next successful flush.
#[test]
fn test_discard_after_a_partial_flush_keeps_every_flushed_byte() {
    use std::io::Write;
    let temp_dir = TempDir::new().expect("temp dir");
    let path = temp_dir.path().join("wal");
    // 10 bytes flushed before, 7 of 20 buffered bytes flushed by the failed
    // append, 13 still buffered: the tracked offset is 10 + 20.
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&[CRC_STORE_MARKER; 17]))
        .expect("write the flushed prefix");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open");

    discard_torn_record(&file, 13, 30).expect("discard");

    assert_eq!(file.metadata().expect("metadata").len(), 17);
}

/// With nothing buffered, every byte past the tracked offset is a fragment
/// of the failed record and is cut.
#[test]
fn test_discard_with_an_empty_buffer_cuts_the_fragment() {
    use std::io::Write;
    let temp_dir = TempDir::new().expect("temp dir");
    let path = temp_dir.path().join("wal");
    std::fs::File::create(&path)
        .and_then(|mut f| f.write_all(&[CRC_STORE_MARKER; 45]))
        .expect("write records plus a fragment");
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open");

    discard_torn_record(&file, 0, 30).expect("discard");

    assert_eq!(file.metadata().expect("metadata").len(), 30);
}
