use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use parking_lot::{Condvar, Mutex};

use super::{LiveCutover, LiveGenerationSlot, LiveRecovery};
use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::migration::{
    commit_retained_switch, finalize_staged_live_switch, prepare_live_switch, stage_live_switch,
    target_embedder_witness,
};
use crate::mutation::controller::{
    ControllerConfig, ControllerPhase, ConvergenceController, ConvergenceSample,
};
use crate::mutation::journal::{DirtyJournal, EpochIdentity};
use crate::mutation::{DirtyKey, MutationObserver};
use crate::storage::NativeStore;
use crate::{HashEmbedder, MemoryService};

#[test]
fn replacement_waits_for_inflight_generation_and_swaps_the_pair() {
    let root = tempfile::tempdir().expect("root");
    let source =
        MemoryService::open(root.path().join("source"), HashEmbedder::new(2)).expect("source");
    let slot = Arc::new(LiveGenerationSlot::new(source, "source-model"));
    let entered = mpsc::sync_channel(0);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let reader_slot = Arc::clone(&slot);
    let reader_release = Arc::clone(&release);
    let reader = std::thread::spawn(move || {
        reader_slot
            .with_generation(|generation| {
                entered.0.send(()).expect("signal entered");
                let mut released = reader_release.0.lock();
                while !*released {
                    reader_release.1.wait(&mut released);
                }
                assert_eq!(generation.model(), "source-model");
                assert_eq!(generation.dimension(), 2);
            })
            .expect("source generation");
    });
    entered.1.recv().expect("reader entered");

    let target =
        MemoryService::open(root.path().join("target"), HashEmbedder::new(3)).expect("target");
    let writer_slot = Arc::clone(&slot);
    let (replaced_tx, replaced_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        writer_slot.replace_for_test(target, "target-model");
        replaced_tx.send(()).expect("signal replaced");
    });

    assert!(replaced_rx.recv_timeout(Duration::from_millis(50)).is_err());
    *release.0.lock() = true;
    release.1.notify_all();
    reader.join().expect("reader");
    writer.join().expect("writer");
    slot.with_generation(|generation| {
        assert_eq!(generation.model(), "target-model");
        assert_eq!(generation.dimension(), 3);
    })
    .expect("target generation");
}

#[test]
fn dropping_both_native_handles_allows_the_two_rename_open() {
    let root = tempfile::tempdir().expect("root");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let archive = root.path().join("source.archive");
    let service = MemoryService::open(&source, HashEmbedder::new(2)).expect("source");
    let target = NativeStore::open(&destination, 3).expect("destination");
    drop(service);
    drop(target);
    std::fs::rename(&source, &archive).expect("archive");
    std::fs::rename(&destination, &source).expect("activate");
    drop(NativeStore::open(&source, 3).expect("open activated target"));
}

#[test]
fn live_cutover_installs_target_and_persists_the_measured_window() {
    let rig = CutoverRig::new();
    let mut controller = rig.quiescing_controller();
    rig.slot
        .cut_over(
            LiveCutover {
                controller: &mut controller,
                journal: &rig.journal,
                source: &rig.source,
                destination: &rig.destination,
                target_model: "target-model",
                started_at: Duration::from_secs(3),
                now: &successful_completion,
            },
            HashEmbedder::new(3),
            |_, _| Ok(()),
        )
        .expect("cut over");

    assert_eq!(controller.phase(), ControllerPhase::Activated);
    assert_eq!(
        controller.measured_cutover(),
        Some(Duration::from_millis(125))
    );
    rig.slot
        .with_generation(|generation| {
            assert_eq!(generation.model(), "target-model");
            assert_eq!(generation.dimension(), 3);
        })
        .expect("target generation");
    assert!(!rig.source.with_file_name("source.archive").exists());
}

#[test]
fn deadline_expiry_reopens_the_source_without_activation() {
    let rig = CutoverRig::new();
    let mut controller = rig.quiescing_controller();
    let error = rig
        .slot
        .cut_over(
            LiveCutover {
                controller: &mut controller,
                journal: &rig.journal,
                source: &rig.source,
                destination: &rig.destination,
                target_model: "target-model",
                started_at: Duration::from_secs(3),
                now: &expired_completion,
            },
            HashEmbedder::new(3),
            |_, _| Ok(()),
        )
        .expect_err("deadline must expire");

    assert!(error.to_string().contains("deadline"), "{error}");
    assert_eq!(controller.phase(), ControllerPhase::CatchingUp);
    rig.slot
        .with_generation(|generation| {
            assert_eq!(generation.model(), "source-model");
            assert_eq!(generation.dimension(), 2);
        })
        .expect("source reopened");
    assert!(rig.source.exists());
    assert!(rig.destination.exists());
    assert!(!rig.source.with_file_name("source.archive").exists());
}

#[test]
fn completion_clock_is_sampled_after_the_destination_is_sealed() {
    let rig = CutoverRig::new();
    let mut controller = rig.quiescing_controller();
    let sealed = AtomicBool::new(false);
    let now = || {
        assert!(
            sealed.load(Ordering::Acquire),
            "completion sampled before seal"
        );
        Duration::from_millis(3_125)
    };
    rig.slot
        .cut_over(
            LiveCutover {
                controller: &mut controller,
                journal: &rig.journal,
                source: &rig.source,
                destination: &rig.destination,
                target_model: "target-model",
                started_at: Duration::from_secs(3),
                now: &now,
            },
            HashEmbedder::new(3),
            |_, _| {
                sealed.store(true, Ordering::Release);
                Ok(())
            },
        )
        .expect("cut over after seal");
}

#[test]
fn dirty_final_watermark_refuses_before_moving_or_retiring_source() {
    let rig = CutoverRig::new();
    rig.journal
        .before_mutation(DirtyKey::Fact(7))
        .expect("dirty record");
    let mut controller = rig.quiescing_controller();
    let error = rig
        .slot
        .cut_over(
            rig.cutover(&mut controller),
            HashEmbedder::new(3),
            |_, _| Ok(()),
        )
        .expect_err("dirty journal must refuse");

    assert!(error.to_string().contains("not drained"), "{error}");
    assert_source_generation(&rig.slot);
    rig.assert_unmoved();
}

#[test]
fn seal_failure_refuses_before_moving_or_retiring_source() {
    let rig = CutoverRig::new();
    let mut controller = rig.quiescing_controller();
    let error = rig
        .slot
        .cut_over(
            rig.cutover(&mut controller),
            HashEmbedder::new(3),
            |_, _| Err(velesdb_core::Error::Query("seal failed".to_owned()).into()),
        )
        .expect_err("seal must fail");

    assert!(error.to_string().contains("seal failed"), "{error}");
    assert_source_generation(&rig.slot);
    rig.assert_unmoved();
}

#[test]
fn source_provenance_mismatch_refuses_before_movement() {
    let rig = CutoverRig::with_active_model("unexpected-source-model");
    let mut controller = rig.quiescing_controller();
    let error = rig
        .slot
        .cut_over(
            rig.cutover(&mut controller),
            HashEmbedder::new(3),
            |_, _| Ok(()),
        )
        .expect_err("source identity must fail");

    assert!(error.to_string().contains("identity"), "{error}");
    rig.assert_unmoved();
}

#[test]
fn restart_while_quiescing_restores_the_staged_source() {
    let rig = CutoverRig::new();
    let controller = rig.quiescing_controller();
    rig.prepare_switch();
    let crashed = rig.crash();
    stage_live_switch(&crashed.source, &crashed.destination).expect("stage");
    drop(controller);
    let mut resumed = crashed.resumed_controller();

    let slot = crashed.recover(&mut resumed).expect("recover source");

    assert_eq!(resumed.phase(), ControllerPhase::CatchingUp);
    assert_source_generation(&slot);
    crashed.assert_unmoved();
}

#[test]
fn restart_after_activation_finishes_forward_and_clears_archive() {
    let rig = CutoverRig::new();
    let mut controller = rig.quiescing_controller();
    rig.prepare_switch();
    let crashed = rig.crash();
    stage_live_switch(&crashed.source, &crashed.destination).expect("stage");
    drop(MemoryService::open(&crashed.source, HashEmbedder::new(3)).expect("target proof"));
    controller
        .activate(Duration::from_millis(3_125))
        .expect("activate");
    drop(controller);
    let mut resumed = crashed.resumed_controller();

    let slot = crashed.recover(&mut resumed).expect("recover target");

    assert_eq!(resumed.phase(), ControllerPhase::Activated);
    assert_eq!(resumed.recovery_action(), None);
    assert_target_generation(&slot);
    assert!(!crashed.archive().exists());
}

#[test]
fn restart_after_archive_commit_is_idempotent() {
    let rig = CutoverRig::new();
    let mut controller = rig.quiescing_controller();
    rig.prepare_switch();
    let crashed = rig.crash();
    stage_live_switch(&crashed.source, &crashed.destination).expect("stage");
    drop(MemoryService::open(&crashed.source, HashEmbedder::new(3)).expect("target proof"));
    controller
        .activate(Duration::from_millis(3_125))
        .expect("activate");
    finalize_staged_live_switch(&crashed.source, &crashed.destination).expect("finalize");
    commit_retained_switch(&crashed.source, &crashed.destination).expect("commit");
    drop(controller);
    let mut resumed = crashed.resumed_controller();

    let slot = crashed.recover(&mut resumed).expect("recover committed");

    assert_target_generation(&slot);
    assert_eq!(resumed.recovery_action(), None);
    assert!(!crashed.archive().exists());
}

struct CutoverRig {
    _root: tempfile::TempDir,
    source: std::path::PathBuf,
    destination: std::path::PathBuf,
    workspace: std::path::PathBuf,
    journal: Arc<DirtyJournal>,
    slot: LiveGenerationSlot<HashEmbedder>,
}

impl CutoverRig {
    fn new() -> Self {
        Self::with_active_model("source-model")
    }

    fn with_active_model(active_model: &str) -> Self {
        let root = tempfile::tempdir().expect("root");
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        let workspace = root.path().join("destination.migration-journal");
        let service = MemoryService::open(&source, HashEmbedder::new(2)).expect("source");
        drop(NativeStore::open(&destination, 3).expect("destination"));
        embedding_provenance::write(&destination, &EmbeddingProvenance::new("target-model", 3))
            .expect("target provenance");
        std::fs::create_dir_all(&workspace).expect("workspace");
        let target_witness =
            crate::migration::target_embedder_witness(&HashEmbedder::new(3)).expect("witness");
        let identity = EpochIdentity::for_test(
            source.clone(),
            "source-model",
            "target-model",
            3,
            &target_witness,
            destination.clone(),
            EPOCH,
        );
        let journal = Arc::new(DirtyJournal::open(&workspace, &identity, 1024).expect("journal"));
        Self {
            _root: root,
            source,
            destination,
            workspace,
            journal,
            slot: LiveGenerationSlot::new(service, active_model),
        }
    }

    fn cutover<'a>(&'a self, controller: &'a mut ConvergenceController) -> LiveCutover<'a> {
        LiveCutover {
            controller,
            journal: &self.journal,
            source: &self.source,
            destination: &self.destination,
            target_model: "target-model",
            started_at: Duration::from_secs(3),
            now: &successful_completion,
        }
    }

    fn prepare_switch(&self) {
        let witness = target_embedder_witness(&HashEmbedder::new(3)).expect("witness");
        prepare_live_switch(&self.source, &self.destination, "target-model", 3, &witness)
            .expect("prepare live switch");
    }

    fn assert_unmoved(&self) {
        assert!(self.source.exists());
        assert!(self.destination.exists());
        assert!(!self.source.with_file_name("source.archive").exists());
    }

    fn crash(self) -> RecoveryRig {
        let Self {
            _root: root,
            source,
            destination,
            workspace,
            journal,
            slot,
        } = self;
        drop(slot);
        RecoveryRig {
            _root: root,
            source,
            destination,
            workspace,
            journal,
        }
    }

    fn quiescing_controller(&self) -> ConvergenceController {
        let mut controller =
            ConvergenceController::open(&self.workspace, EPOCH, config()).expect("controller");
        for (second, input, output) in [(0, 100, 80), (1, 110, 100), (2, 120, 120)] {
            controller
                .observe(sample(second, input, output))
                .expect("observe");
        }
        controller
            .begin_quiescing(Duration::from_secs(3))
            .expect("quiesce");
        controller
    }
}

fn successful_completion() -> Duration {
    Duration::from_millis(3_125)
}

fn expired_completion() -> Duration {
    Duration::from_millis(3_501)
}

struct RecoveryRig {
    _root: tempfile::TempDir,
    source: std::path::PathBuf,
    destination: std::path::PathBuf,
    workspace: std::path::PathBuf,
    journal: Arc<DirtyJournal>,
}

impl RecoveryRig {
    fn resumed_controller(&self) -> ConvergenceController {
        let controller =
            ConvergenceController::open(&self.workspace, EPOCH, config()).expect("resume");
        assert!(controller.recovery_action().is_some());
        controller
    }

    fn recover(
        &self,
        controller: &mut ConvergenceController,
    ) -> Result<LiveGenerationSlot<HashEmbedder>, crate::MemoryError> {
        LiveGenerationSlot::recover(
            LiveRecovery {
                controller,
                journal: &self.journal,
                source: &self.source,
                destination: &self.destination,
                source_model: "source-model",
                target_model: "target-model",
            },
            HashEmbedder::new(2),
            HashEmbedder::new(3),
        )
    }

    fn archive(&self) -> std::path::PathBuf {
        self.source.with_file_name("source.archive")
    }

    fn assert_unmoved(&self) {
        assert!(self.source.exists());
        assert!(self.destination.exists());
        assert!(!self.archive().exists());
    }
}

fn assert_source_generation(slot: &LiveGenerationSlot<HashEmbedder>) {
    slot.with_generation(|generation| {
        assert_eq!(generation.model(), "source-model");
        assert_eq!(generation.dimension(), 2);
    })
    .expect("source generation");
}

fn assert_target_generation(slot: &LiveGenerationSlot<HashEmbedder>) {
    slot.with_generation(|generation| {
        assert_eq!(generation.model(), "target-model");
        assert_eq!(generation.dimension(), 3);
    })
    .expect("target generation");
}

const EPOCH: &str = "00112233445566778899aabbccddeeff";

fn config() -> ControllerConfig {
    ControllerConfig {
        observation_window: 3,
        pause_budget: Duration::from_millis(500),
        verification_reserve: Duration::from_millis(50),
    }
}

fn sample(second: u64, input: u64, output: u64) -> ConvergenceSample {
    ConvergenceSample {
        observed_at: Duration::from_secs(second),
        input_watermark: input,
        output_watermark: output,
        distinct_dirty_facts: 2,
        distinct_edge_sources: 1,
        pending_journal_bytes: (input - output) * 49,
        replay_elapsed: Duration::from_millis(20),
        largest_apply_latency: Duration::from_millis(20),
    }
}
