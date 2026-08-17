use std::time::Duration;

use super::job_state::{JobPhase, JobRecord, JobSpec, JobStore};
use super::startup::recover_startup;
use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::migration::{
    prepare_live_switch, stage_live_switch, target_embedder_witness, OnlineMigrationStartup,
};
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::{ControllerConfig, ConvergenceController, ConvergenceSample};
use crate::mutation::journal::{DirtyJournal, EpochIdentity};
use crate::storage::NativeStore;
use crate::{DynEmbedder, HashEmbedder, MemoryService};

const EPOCH: &str = "00112233445566778899aabbccddeeff";

#[test]
fn startup_recovers_when_controller_quiescing_precedes_job_state() {
    let rig = StartupRig::new();
    drop(rig.quiescing_controller());

    let recovery = recover_startup(&rig.source, |_| {
        panic!("target factory must not run for source rollback")
    })
    .expect("startup recovery");

    assert!(matches!(
        recovery,
        OnlineMigrationStartup::SourceRestored { ref source_model }
            if source_model == "source-model"
    ));
    assert_eq!(rig.store.load().expect("job").phase, JobPhase::CatchingUp);
    assert!(rig.source.exists());
    assert!(rig.destination.exists());
}

#[test]
fn startup_finishes_an_activated_job_forward_after_witness_verification() {
    let rig = StartupRig::new();
    let mut controller = rig.quiescing_controller();
    let witness = target_embedder_witness(&HashEmbedder::new(3)).expect("witness");
    prepare_live_switch(&rig.source, &rig.destination, "target-model", 3, &witness)
        .expect("prepare switch");
    stage_live_switch(&rig.source, &rig.destination).expect("stage switch");
    controller
        .activate(Duration::from_millis(1_010))
        .expect("activate controller");

    let recovery = recover_startup(&rig.source, |backend| {
        assert_eq!(backend, "hash-3");
        Ok((
            Box::new(HashEmbedder::new(3)) as DynEmbedder,
            "target-model".to_owned(),
        ))
    })
    .expect("startup recovery");

    assert!(matches!(
        recovery,
        OnlineMigrationStartup::TargetActivated { ref model, .. }
            if model == "target-model"
    ));
    assert_eq!(rig.store.load().expect("job").phase, JobPhase::Committed);
    assert!(rig.source.exists());
    assert!(!rig.destination.exists());
    assert!(!rig.source.with_file_name("source.archive").exists());
}

struct StartupRig {
    _root: tempfile::TempDir,
    source: std::path::PathBuf,
    destination: std::path::PathBuf,
    workspace: std::path::PathBuf,
    store: JobStore,
    config: ControllerConfig,
}

impl StartupRig {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("root");
        let source = root.path().join("source");
        let destination = root.path().join("source.online-migration-target");
        let workspace = root
            .path()
            .join("source.online-migration-target.migration-journal");
        let control = root.path().join("source.online-migration-control");
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::create_dir_all(&control).expect("control");
        drop(MemoryService::open(&source, HashEmbedder::new(2)).expect("source"));
        drop(NativeStore::open(&destination, 3).expect("destination"));
        embedding_provenance::write(&destination, &EmbeddingProvenance::new("target-model", 3))
            .expect("provenance");
        let identity = identity(&source, &destination);
        drop(DirtyJournal::open(&workspace, &identity, 1024 * 1024).expect("journal"));
        let config = controller_config();
        let record = quiescing_record(identity, workspace.clone(), config);
        let store = JobStore::create(&control, &record).expect("job store");
        Self {
            _root: root,
            source,
            destination,
            workspace,
            store,
            config,
        }
    }

    fn quiescing_controller(&self) -> ConvergenceController {
        let mut controller =
            ConvergenceController::open(&self.workspace, EPOCH, self.config).expect("controller");
        controller.observe(sample(0)).expect("first");
        controller.observe(sample(1)).expect("second");
        controller
            .begin_quiescing(Duration::from_secs(1))
            .expect("quiesce");
        controller
    }
}

fn identity(source: &std::path::Path, destination: &std::path::Path) -> EpochIdentity {
    EpochIdentity::for_test(
        source.to_owned(),
        "source-model",
        "target-model",
        3,
        &target_embedder_witness(&HashEmbedder::new(3)).expect("witness"),
        destination.to_owned(),
        EPOCH,
    )
}

fn controller_config() -> ControllerConfig {
    ControllerConfig {
        observation_window: 2,
        pause_budget: Duration::from_secs(1),
        verification_reserve: Duration::from_millis(10),
    }
}

fn quiescing_record(
    identity: EpochIdentity,
    workspace: std::path::PathBuf,
    controller: ControllerConfig,
) -> JobRecord {
    let mut record = JobRecord::new(JobSpec {
        identity,
        target_backend: "hash-3".to_owned(),
        journal_max_bytes: 1024 * 1024,
        catch_up: CatchUpConfig {
            fact_batch: 16,
            replay_batch: 16,
            edge_cap: 16,
        },
        controller,
        workspace,
    })
    .expect("job");
    advance_to_cutover_ready(&mut record);
    record
}

fn advance_to_cutover_ready(record: &mut JobRecord) {
    for phase in [
        JobPhase::Capturing,
        JobPhase::BaseCopied,
        JobPhase::CatchingUp,
        JobPhase::CutoverReady,
    ] {
        record.transition(phase).expect("transition");
    }
}

fn sample(second: u64) -> ConvergenceSample {
    ConvergenceSample {
        observed_at: Duration::from_secs(second),
        input_watermark: 0,
        output_watermark: 0,
        distinct_dirty_facts: 0,
        distinct_edge_sources: 0,
        pending_journal_bytes: 0,
        replay_elapsed: Duration::from_millis(1),
        largest_apply_latency: Duration::ZERO,
    }
}
