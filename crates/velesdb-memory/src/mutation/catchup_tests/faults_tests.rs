use serde_json::json;

use super::{journal, FixedEmbedder, TestRig};
use crate::mutation::catchup::{CatchUpConfig, FaultPoint};
use crate::storage::NativeStore;
use crate::{MemoryService, Metadata};

#[test]
fn fact_and_pre_watermark_faults_leave_replay_unacknowledged() {
    for point in [FaultPoint::AfterFact, FaultPoint::BeforeWatermark] {
        assert_fact_fault_retries(point, false);
    }
}

#[test]
fn edge_fault_leaves_replay_unacknowledged_and_retriable() {
    let rig = TestRig::new();
    let from = rig.source.remember("from", &[], None).expect("from");
    let to = rig.source.remember("to", &[], None).expect("to");
    let copy = rig.start();
    copy.copy_base().expect("base copy");
    rig.source.relate(from, to, "uses").expect("edge");
    copy.fail_once_at(FaultPoint::AfterEdges);

    copy.catch_up_batch().expect_err("injected edge fault");
    assert_eq!(rig.journal.compacted_through(), 0);
    let retry = copy.catch_up_batch().expect("retry");
    assert_eq!(retry.records, 1);
    assert_eq!(retry.backlog, 0);
    assert_eq!(
        rig.destination
            .migration_live_edges(from, 8)
            .expect("destination edge"),
        rig.source
            .migration_store()
            .migration_live_edges(from, 8)
            .expect("source edge")
    );
    copy.finish().expect("finish");
}

#[test]
fn post_watermark_fault_reports_error_but_does_not_reapply_records() {
    assert_fact_fault_retries(FaultPoint::AfterWatermark, true);
}

#[test]
fn migration_work_limits_are_positive_and_capped() {
    for invalid in [0, 4_097] {
        let config = CatchUpConfig {
            fact_batch: invalid,
            replay_batch: 1,
            edge_cap: 1,
        };
        assert!(config.validated().is_err());
    }
    assert!(CatchUpConfig {
        fact_batch: 4_096,
        replay_batch: 4_096,
        edge_cap: 4_096,
    }
    .validated()
    .is_ok());
}

#[test]
fn reopening_after_an_unacknowledged_replay_converges_idempotently() {
    let root = tempfile::tempdir().expect("root");
    let source_path = root.path().join("source");
    let destination_path = root.path().join("destination");
    let journal_path = root.path().join("journal");
    let source = MemoryService::open(
        &source_path,
        FixedEmbedder {
            vector: vec![1.0, 2.0],
        },
    )
    .expect("source");
    let id = source.remember("alpha", &[], None).expect("alpha");
    let destination = NativeStore::open(&destination_path, 3).expect("destination");
    let first_journal = journal(&journal_path, &source_path, &destination_path);
    let target = FixedEmbedder {
        vector: vec![7.0, 8.0, 9.0],
    };
    let copy = crate::mutation::catchup::OnlineCatchUp::start(
        &source,
        &destination,
        &target,
        first_journal.clone(),
        config(),
    )
    .expect("start");
    copy.copy_base().expect("base copy");
    let mut metadata = Metadata::new();
    metadata.insert("version".to_owned(), json!(2));
    source
        .remember("alpha", &[], Some(&metadata))
        .expect("overwrite");
    copy.fail_once_at(FaultPoint::BeforeWatermark);
    copy.catch_up_batch().expect_err("unacknowledged fault");
    drop(copy);
    drop(first_journal);
    drop(destination);
    drop(source);

    let source = MemoryService::open(
        &source_path,
        FixedEmbedder {
            vector: vec![1.0, 2.0],
        },
    )
    .expect("reopen source");
    let destination = NativeStore::open(&destination_path, 3).expect("reopen destination");
    let resumed = crate::mutation::catchup::OnlineCatchUp::start(
        &source,
        &destination,
        &target,
        journal(&journal_path, &source_path, &destination_path),
        config(),
    )
    .expect("resume");
    resumed.copy_base().expect("repeat base copy");
    assert_eq!(resumed.catch_up_batch().expect("replay").records, 1);
    assert_eq!(
        destination.migration_payload(id).expect("destination"),
        source
            .migration_store()
            .migration_payload(id)
            .expect("source")
    );
    resumed.finish().expect("finish");
}

fn assert_fact_fault_retries(point: FaultPoint, acknowledged: bool) {
    let rig = TestRig::new();
    let id = rig.source.remember("alpha", &[], None).expect("alpha");
    let copy = rig.start();
    copy.copy_base().expect("base copy");
    let mut metadata = Metadata::new();
    metadata.insert("version".to_owned(), json!(2));
    rig.source
        .remember("alpha", &[], Some(&metadata))
        .expect("overwrite");
    copy.fail_once_at(point);

    copy.catch_up_batch().expect_err("injected fault");
    assert_eq!(rig.journal.compacted_through() > 0, acknowledged);
    let retry = copy.catch_up_batch().expect("retry");
    assert_eq!(retry.records == 0, acknowledged);
    assert_eq!(retry.backlog, 0);
    assert_eq!(
        rig.destination
            .migration_payload(id)
            .expect("destination payload"),
        rig.source
            .migration_store()
            .migration_payload(id)
            .expect("source payload")
    );
    copy.finish().expect("finish");
}

fn config() -> CatchUpConfig {
    CatchUpConfig {
        fact_batch: 8,
        replay_batch: 8,
        edge_cap: 8,
    }
}
