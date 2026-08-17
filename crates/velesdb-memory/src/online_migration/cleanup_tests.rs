use std::time::Duration;

use super::{remove_cancelled_artifacts, JobPhase, JobRecord, JobSpec};
use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::ControllerConfig;
use crate::mutation::journal::{DirtyJournal, EpochIdentity};
use crate::storage::NativeStore;

#[test]
fn cancellation_refuses_foreign_target_provenance_without_removing_evidence() {
    let root = tempfile::tempdir().expect("root");
    let record = cancelled_record(root.path());
    drop(NativeStore::open(record.spec.identity.destination_path(), 3).expect("destination"));
    embedding_provenance::write(
        record.spec.identity.destination_path(),
        &EmbeddingProvenance::new("foreign-model", 3),
    )
    .expect("foreign provenance");

    let error = remove_cancelled_artifacts(&record).expect_err("foreign target must refuse");

    assert!(error.to_string().contains("provenance"), "{error}");
    assert!(record.spec.identity.destination_path().exists());
    assert!(record.spec.workspace.exists());
}

#[cfg(unix)]
#[test]
fn cancellation_never_follows_a_destination_symlink() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let record = cancelled_record(root.path());
    let foreign = root.path().join("foreign");
    std::fs::create_dir(&foreign).expect("foreign");
    std::fs::write(foreign.join("keep"), b"evidence").expect("marker");
    symlink(&foreign, record.spec.identity.destination_path()).expect("symlink");

    remove_cancelled_artifacts(&record).expect_err("symlink must refuse");

    assert_eq!(
        std::fs::read(foreign.join("keep")).expect("marker"),
        b"evidence"
    );
    assert!(record.spec.workspace.exists());
}

fn cancelled_record(root: &std::path::Path) -> JobRecord {
    let destination = root.join("source.online-migration-target");
    let workspace = root.join("source.online-migration-target.migration-journal");
    std::fs::create_dir(&workspace).expect("workspace");
    let identity = EpochIdentity::for_test(
        root.join("source"),
        "source-model",
        "target-model",
        3,
        &format!("sha256:{}", "ab".repeat(32)),
        destination,
        "00112233445566778899aabbccddeeff",
    );
    drop(DirtyJournal::open(&workspace, &identity, 1024 * 1024).expect("journal"));
    let mut record = JobRecord::new(JobSpec {
        identity,
        target_backend: "hash".to_owned(),
        journal_max_bytes: 1024 * 1024,
        catch_up: CatchUpConfig {
            fact_batch: 8,
            replay_batch: 8,
            edge_cap: 8,
        },
        controller: ControllerConfig {
            observation_window: 2,
            pause_budget: Duration::from_secs(1),
            verification_reserve: Duration::from_millis(10),
        },
        workspace,
    })
    .expect("record");
    record.transition(JobPhase::Cancelled).expect("cancel");
    record
}
