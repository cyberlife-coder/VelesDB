use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use super::journal::{
    DirtyJournal, EpochIdentity, FaultPoint, JournalRecord, JOURNAL_FILE, RECORD_BYTES,
};
use super::{DirtyKey, MutationObserver};
use crate::{HashEmbedder, MemoryService};

const CAPACITY: u64 = 16 * 1024;

fn epoch(root: &Path, id: &str) -> EpochIdentity {
    EpochIdentity::for_test(
        root.join("source"),
        "sha256:source",
        "target-model",
        384,
        root.join("destination"),
        id,
    )
}

#[test]
fn append_is_durable_monotonic_and_recoverable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
    journal.before_mutation(DirtyKey::Fact(7)).expect("fact");
    journal
        .before_mutation(DirtyKey::OutgoingEdges(9))
        .expect("edges");
    drop(journal);

    let reopened = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("reopen");
    assert_eq!(reopened.last_sequence(), 2);
    assert_eq!(
        reopened.records_after(0, 8).expect("records"),
        vec![
            JournalRecord::new(1, DirtyKey::Fact(7)),
            JournalRecord::new(2, DirtyKey::OutgoingEdges(9)),
        ]
    );
}

#[test]
fn identity_mismatch_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    DirtyJournal::open(
        dir.path(),
        &epoch(dir.path(), "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        CAPACITY,
    )
    .expect("create");
    let error = DirtyJournal::open(
        dir.path(),
        &epoch(dir.path(), "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        CAPACITY,
    )
    .err()
    .expect("mismatch");
    assert!(error.to_string().contains("identity mismatch"), "{error}");
}

#[cfg(unix)]
#[test]
fn preexisting_broken_symlink_is_refused_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("outside-target");
    symlink(&target, dir.path().join(JOURNAL_FILE)).expect("symlink");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");

    let error = DirtyJournal::open(dir.path(), &identity, CAPACITY)
        .err()
        .expect("symlink refusal");
    assert!(error.to_string().contains("regular file"), "{error}");
    assert!(!target.exists());
}

#[test]
fn torn_tail_is_truncated_to_the_complete_valid_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
    journal.before_mutation(DirtyKey::Fact(1)).expect("append");
    drop(journal);
    let path = dir.path().join(JOURNAL_FILE);
    let valid_len = std::fs::metadata(&path).expect("metadata").len();
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open append")
        .write_all(&[0x5a; 11])
        .expect("write tail");

    let reopened = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("recover");
    assert_eq!(reopened.last_sequence(), 1);
    assert_eq!(std::fs::metadata(path).expect("metadata").len(), valid_len);
}

#[test]
fn interior_corruption_fails_closed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
    for id in 1..=3 {
        journal.before_mutation(DirtyKey::Fact(id)).expect("append");
    }
    let record_start = journal.header_bytes();
    drop(journal);
    let mut file = OpenOptions::new()
        .write(true)
        .open(dir.path().join(JOURNAL_FILE))
        .expect("open journal");
    file.seek(SeekFrom::Start(record_start + 4)).expect("seek");
    file.write_all(&[0xff]).expect("corrupt");
    file.sync_all().expect("sync");

    let error = DirtyJournal::open(dir.path(), &identity, CAPACITY)
        .err()
        .expect("corruption");
    assert!(error.to_string().contains("interior corruption"), "{error}");
}

#[test]
fn failed_sync_poisoning_prevents_a_source_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal_workspace = dir.path().join("journal");
    std::fs::create_dir(&journal_workspace).expect("journal workspace");
    let journal = Arc::new(
        DirtyJournal::open(
            &journal_workspace,
            &epoch(dir.path(), "00112233445566778899aabbccddeeff"),
            CAPACITY,
        )
        .expect("open"),
    );
    let service = MemoryService::open(dir.path().join("source"), HashEmbedder::new(384))
        .expect("open source");
    service
        .install_mutation_observer(Some(journal.clone()))
        .expect("install journal");
    journal.fail_once_at(FaultPoint::BeforeAppendSync);

    assert!(service.remember("must not land", &[], None).is_err());
    assert_eq!(service.fact_count(), 0);
    assert!(service.remember("still poisoned", &[], None).is_err());
}

#[test]
fn crash_after_source_mutation_leaves_both_source_and_record_durable() {
    let dir = tempfile::tempdir().expect("tempdir");
    let journal_workspace = dir.path().join("journal");
    let source = dir.path().join("source");
    std::fs::create_dir(&journal_workspace).expect("journal workspace");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let journal = Arc::new(
        DirtyJournal::open(&journal_workspace, &identity, CAPACITY).expect("open journal"),
    );
    let service = MemoryService::open(&source, HashEmbedder::new(384)).expect("open source");
    service
        .install_mutation_observer(Some(journal.clone()))
        .expect("install journal");
    service.remember("durable", &[], None).expect("remember");
    drop(service);
    drop(journal);

    let recovered = DirtyJournal::open(&journal_workspace, &identity, CAPACITY).expect("recover");
    let reopened = MemoryService::open(source, HashEmbedder::new(384)).expect("reopen source");
    assert_eq!(recovered.last_sequence(), 1);
    assert_eq!(reopened.fact_count(), 1);
}

#[test]
fn every_append_boundary_fails_closed_and_recovers_a_valid_prefix() {
    let points = [
        FaultPoint::BeforeAppend,
        FaultPoint::AfterAppend,
        FaultPoint::BeforeAppendSync,
        FaultPoint::AfterAppendSync,
    ];
    for (index, point) in points.into_iter().enumerate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
        let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
        journal.fail_once_at(point);
        assert!(journal
            .before_mutation(DirtyKey::Fact(index as u64))
            .is_err());
        assert!(journal.before_mutation(DirtyKey::Fact(99)).is_err());
        drop(journal);

        let reopened = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("recover");
        assert!(reopened.last_sequence() <= 1);
        reopened
            .before_mutation(DirtyKey::Fact(100))
            .expect("resume");
    }
}

#[test]
fn disk_cap_refuses_before_an_unjournalled_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let probe = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("probe");
    let one_record_cap = probe.header_bytes() + RECORD_BYTES;
    drop(probe);
    let journal = DirtyJournal::open(dir.path(), &identity, one_record_cap).expect("reopen");
    journal.before_mutation(DirtyKey::Fact(1)).expect("first");
    let error = journal.before_mutation(DirtyKey::Fact(2)).expect_err("cap");
    assert!(error.to_string().contains("byte cap"), "{error}");
    assert_eq!(journal.last_sequence(), 1);
    journal.compact_through(1).expect("compact at cap");
    journal
        .before_mutation(DirtyKey::Fact(2))
        .expect("append after compaction");
    assert_eq!(journal.last_sequence(), 2);
}

#[test]
fn compaction_streams_unacknowledged_records_and_preserves_sequence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
    for id in 1..=4 {
        journal.before_mutation(DirtyKey::Fact(id)).expect("append");
    }
    journal.compact_through(3).expect("compact");
    assert_eq!(journal.compacted_through(), 3);
    assert_eq!(journal.last_sequence(), 4);
    assert_eq!(journal.records_after(0, 8).expect("records").len(), 1);
    journal
        .before_mutation(DirtyKey::OutgoingEdges(5))
        .expect("append next");
    drop(journal);

    let reopened = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("reopen");
    assert_eq!(reopened.last_sequence(), 5);
    assert_eq!(reopened.compacted_through(), 3);
}

#[test]
fn interrupted_compaction_keeps_the_authoritative_generation() {
    let dir = tempfile::tempdir().expect("tempdir");
    let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
    let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
    for id in 1..=3 {
        journal.before_mutation(DirtyKey::Fact(id)).expect("append");
    }
    journal.fail_once_at(FaultPoint::BeforeCompactionReplace);
    assert!(journal.compact_through(2).is_err());
    drop(journal);

    let reopened = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("recover");
    assert_eq!(reopened.last_sequence(), 3);
    assert_eq!(reopened.records_after(0, 8).expect("records").len(), 3);
}

#[test]
fn every_compaction_boundary_preserves_all_unacknowledged_records() {
    let points = [
        FaultPoint::BeforeCompactionSync,
        FaultPoint::AfterCompactionSync,
        FaultPoint::BeforeCompactionReplace,
        FaultPoint::AfterCompactionReplace,
        FaultPoint::BeforeDirectorySync,
        FaultPoint::AfterDirectorySync,
    ];
    for point in points {
        let dir = tempfile::tempdir().expect("tempdir");
        let identity = epoch(dir.path(), "00112233445566778899aabbccddeeff");
        let journal = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("open");
        for id in 1..=3 {
            journal.before_mutation(DirtyKey::Fact(id)).expect("append");
        }
        journal.fail_once_at(point);
        assert!(journal.compact_through(2).is_err());
        drop(journal);

        let reopened = DirtyJournal::open(dir.path(), &identity, CAPACITY).expect("recover");
        assert_eq!(reopened.last_sequence(), 3);
        let pending = reopened.records_after(2, 8).expect("pending");
        assert_eq!(pending, vec![JournalRecord::new(3, DirtyKey::Fact(3))]);
    }
}

#[test]
fn generated_epoch_ids_are_random_and_well_formed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = EpochIdentity::new(
        dir.path().join("source"),
        "sha256:source".to_owned(),
        "target-model".to_owned(),
        384,
        dir.path().join("destination"),
    )
    .expect("first epoch");
    let second = EpochIdentity::new(
        dir.path().join("source"),
        "sha256:source".to_owned(),
        "target-model".to_owned(),
        384,
        dir.path().join("destination"),
    )
    .expect("second epoch");
    assert_ne!(first, second);
}
