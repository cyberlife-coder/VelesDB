use std::path::PathBuf;
use std::time::Duration;

use super::job_state::{JobPhase, JobRecord, JobSpec, JobStore};
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::ControllerConfig;
use crate::mutation::journal::EpochIdentity;

const EPOCH: &str = "0123456789abcdef0123456789abcdef";
const JOB_FILE: &str = "online-migration-job.json";
#[cfg(unix)]
const STAGING_FILE: &str = "online-migration-job.json.tmp";

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
    let path = workspace.join(JOB_FILE);
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

#[cfg(unix)]
#[test]
fn a_symlinked_job_file_is_refused_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"untouched").expect("victim");
    symlink(&victim, workspace.join(JOB_FILE)).expect("symlink");

    let errors = [
        JobStore::try_open(&workspace).err().expect("try_open"),
        JobStore::open(&workspace).err().expect("open"),
        JobStore::create(&workspace, &record(root.path()))
            .err()
            .expect("create"),
    ];
    for error in errors {
        assert!(
            error
                .to_string()
                .contains("online migration job path must be a regular file"),
            "{error}"
        );
    }
    assert_eq!(std::fs::read(&victim).expect("victim"), b"untouched");
}

#[cfg(unix)]
#[test]
fn a_symlinked_staging_file_is_refused_and_left_in_place() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let workspace = root.path().join("workspace");
    std::fs::create_dir(&workspace).expect("workspace");
    let victim = root.path().join("victim");
    std::fs::write(&victim, b"untouched").expect("victim");
    let staging = workspace.join(STAGING_FILE);
    symlink(&victim, &staging).expect("symlink");

    let errors = [
        JobStore::create(&workspace, &record(root.path()))
            .err()
            .expect("create"),
        JobStore::try_open(&workspace).err().expect("try_open"),
        JobStore::open(&workspace).err().expect("open"),
    ];
    for error in errors {
        assert!(
            error
                .to_string()
                .contains("online migration job path must be a regular file"),
            "{error}"
        );
        assert_eq!(std::fs::read(&victim).expect("victim"), b"untouched");
        assert!(std::fs::symlink_metadata(&staging)
            .expect("the link is still there")
            .file_type()
            .is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_workspace_is_refused_by_every_entry_point() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("root");
    let real = root.path().join("real");
    std::fs::create_dir(&real).expect("real");
    let link = root.path().join("link");
    symlink(&real, &link).expect("symlink");

    let errors = [
        JobStore::create(&link, &record(root.path()))
            .err()
            .expect("create"),
        JobStore::try_open(&link).err().expect("try_open"),
        JobStore::open(&link).err().expect("open"),
    ];
    for error in errors {
        assert!(
            error
                .to_string()
                .contains("online migration workspace must be a real directory"),
            "{error}"
        );
    }
    assert_eq!(std::fs::read_dir(&real).expect("real").count(), 0);
}

#[test]
fn a_missing_workspace_names_the_online_migration_store() {
    let root = tempfile::tempdir().expect("root");
    let absent = root.path().join("absent");
    let errors = [
        JobStore::create(&absent, &record(root.path()))
            .err()
            .expect("create"),
        JobStore::try_open(&absent).err().expect("try_open"),
        JobStore::open(&absent).err().expect("open"),
    ];
    for error in errors {
        assert!(
            error
                .to_string()
                .contains("cannot inspect online migration workspace: "),
            "{error}"
        );
    }
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
