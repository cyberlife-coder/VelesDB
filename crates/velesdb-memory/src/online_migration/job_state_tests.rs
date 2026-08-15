use std::path::PathBuf;
use std::time::Duration;

use super::job_state::{JobPhase, JobRecord, JobSpec, JobStore};
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::ControllerConfig;
use crate::mutation::journal::EpochIdentity;

const EPOCH: &str = "0123456789abcdef0123456789abcdef";

#[test]
fn durable_job_round_trips_its_complete_resume_contract() {
    let root = tempfile::tempdir().expect("root");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    let record = record(root.path());

    let store = JobStore::create(&workspace, &record).expect("create job");
    let loaded = store.load().expect("load job");

    assert_eq!(loaded, record);
}

#[test]
fn future_job_version_is_refused_instead_of_guessed() {
    let root = tempfile::tempdir().expect("root");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    let record = record(root.path());
    let store = JobStore::create(&workspace, &record).expect("create job");
    let path = workspace.join("online-migration-job.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("read state")).expect("json");
    value["version"] = serde_json::json!(99);
    std::fs::write(&path, serde_json::to_vec(&value).expect("encode")).expect("write future");

    let error = store.load().expect_err("future version must refuse");

    assert!(error.to_string().contains("version"), "{error}");
}

#[test]
fn phase_machine_allows_deadline_reopen_but_refuses_unsafe_rollback() {
    let root = tempfile::tempdir().expect("root");
    let mut record = record(root.path());
    for phase in [
        JobPhase::Capturing,
        JobPhase::BaseCopied,
        JobPhase::CatchingUp,
        JobPhase::CutoverReady,
        JobPhase::Quiescing,
        JobPhase::CatchingUp,
    ] {
        record.transition(phase).expect("valid transition");
    }
    record
        .transition(JobPhase::CutoverReady)
        .expect("ready again");
    record
        .transition(JobPhase::Quiescing)
        .expect("quiescing again");
    record.transition(JobPhase::Activated).expect("activate");

    let error = record
        .transition(JobPhase::Cancelled)
        .expect_err("activated job cannot cancel");

    assert!(error.to_string().contains("transition"), "{error}");
}

#[test]
fn pre_quiescing_job_can_cancel_but_cannot_restart() {
    let root = tempfile::tempdir().expect("root");
    let mut record = record(root.path());
    record.transition(JobPhase::Capturing).expect("capture");
    record
        .transition(JobPhase::Cancelled)
        .expect("cancel source-authoritative job");

    let error = record
        .transition(JobPhase::CatchingUp)
        .expect_err("terminal cancellation");

    assert!(error.to_string().contains("transition"), "{error}");
}

fn record(root: &std::path::Path) -> JobRecord {
    let source = root.join("source");
    let destination = root.join("destination");
    let identity = EpochIdentity::for_test(
        source,
        "source-model",
        "target-model",
        3,
        &format!("sha256:{}", "ab".repeat(32)),
        destination,
        EPOCH,
    );
    JobRecord::new(JobSpec {
        identity,
        target_backend: "hash".to_owned(),
        journal_max_bytes: 1_048_576,
        catch_up: CatchUpConfig {
            fact_batch: 64,
            replay_batch: 64,
            edge_cap: 64,
        },
        controller: ControllerConfig {
            observation_window: 3,
            pause_budget: Duration::from_secs(1),
            verification_reserve: Duration::from_millis(50),
        },
        workspace: PathBuf::from("workspace"),
    })
    .expect("record")
}
