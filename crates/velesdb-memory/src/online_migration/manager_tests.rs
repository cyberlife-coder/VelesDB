use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::job_runner::JobTarget;
use super::job_state::JobPhase;
use super::manager::{MigrationStartConfig, OnlineMigrationManager};
use super::LiveGenerationSlot;
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::ControllerConfig;
use crate::{DynEmbedder, EmbedError, Embedder, HashEmbedder, MemoryService};

const FUNCTIONAL_TIMEOUT: Duration = Duration::from_secs(30);

#[test]
fn manager_starts_in_background_and_reports_durable_completion() {
    let root = tempfile::tempdir().expect("root");
    let source = root.path().join("source");
    let source_embedder: DynEmbedder = Box::new(HashEmbedder::new(2));
    let service = MemoryService::open(&source, source_embedder).expect("source");
    let slot = Arc::new(LiveGenerationSlot::new(service, "source-model"));
    let factory = Arc::new(|backend: &str| {
        if backend != "hash-3" {
            return Err(crate::MemoryError::MigrationCapture(
                "unsupported test backend".to_owned(),
            ));
        }
        Ok(JobTarget {
            embedder: Box::new(HashEmbedder::new(3)) as DynEmbedder,
            model: "target-model".to_owned(),
        })
    });
    let manager = OnlineMigrationManager::new(Arc::clone(&slot), source, factory).expect("manager");

    let accepted = manager.start("hash-3", config()).expect("start migration");
    assert_eq!(accepted.phase, JobPhase::Prepared);
    let deadline = Instant::now() + FUNCTIONAL_TIMEOUT;
    loop {
        let status = manager.status().expect("status").expect("job");
        if status.record.phase == JobPhase::Committed {
            assert!(status.record.progress.measured_cutover.is_some());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "migration did not finish: {:?}",
            status.record.phase
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    slot.inspect_active(|model, dimension, _| {
        assert_eq!(model, "target-model");
        assert_eq!(dimension, 3);
    })
    .expect("target generation");
}

struct BlockingTarget {
    entered: mpsc::Sender<()>,
    release: Arc<(parking_lot::Mutex<bool>, parking_lot::Condvar)>,
    calls: std::sync::atomic::AtomicUsize,
}

impl Embedder for BlockingTarget {
    fn dimension(&self) -> usize {
        3
    }

    fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbedError> {
        if self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            return Ok(vec![0.0; 3]);
        }
        self.entered.send(()).expect("entered");
        let mut released = self.release.0.lock();
        while !*released {
            self.release.1.wait(&mut released);
        }
        Ok(vec![0.0; 3])
    }
}

struct FailAfterWitness {
    calls: std::sync::atomic::AtomicUsize,
    inner: HashEmbedder,
}

impl Embedder for FailAfterWitness {
    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        if self
            .calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            == 0
        {
            return self.inner.embed(text);
        }
        Err(EmbedError::Backend("injected base-copy failure".to_owned()))
    }
}

#[test]
fn cancellation_is_durable_cleans_up_and_allows_replacement() {
    let root = tempfile::tempdir().expect("root");
    let source = root.path().join("source");
    let source_embedder: DynEmbedder = Box::new(HashEmbedder::new(2));
    let service = MemoryService::open(&source, source_embedder).expect("source");
    service.remember("one", &[], None).expect("fact");
    let slot = Arc::new(LiveGenerationSlot::new(service, "source-model"));
    let (entered_tx, entered_rx) = mpsc::channel();
    let release = Arc::new((parking_lot::Mutex::new(false), parking_lot::Condvar::new()));
    let factory_release = Arc::clone(&release);
    let factory = Arc::new(move |_backend: &str| {
        Ok(JobTarget {
            embedder: Box::new(BlockingTarget {
                entered: entered_tx.clone(),
                release: Arc::clone(&factory_release),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }) as DynEmbedder,
            model: "target-model".to_owned(),
        })
    });
    let manager = OnlineMigrationManager::new(Arc::clone(&slot), source, factory).expect("manager");
    manager.start("blocking", config()).expect("start");
    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("base copy entered target");

    let requested = manager.cancel().expect("request cancellation");
    assert!(requested.cancellation_requested);
    *release.0.lock() = true;
    release.1.notify_all();
    wait_for(&manager, |status| {
        status.record.phase == JobPhase::Cancelled
            && status.record.cancellation_requested
            && !status.running
    });
    assert_generation(&slot, "source-model", 2);
    assert_cancelled_artifacts_removed(root.path());
    manager.start("blocking", config()).expect("restart");
    wait_for(&manager, |status| {
        status.record.phase == JobPhase::Committed
    });
    assert_generation(&slot, "target-model", 3);
}

fn assert_generation(
    slot: &LiveGenerationSlot<DynEmbedder>,
    expected_model: &str,
    expected_dimension: usize,
) {
    slot.inspect_active(|model, dimension, service| {
        assert_eq!(model, expected_model);
        assert_eq!(dimension, expected_dimension);
        assert!(!service.migration_capture_active());
    })
    .expect("active generation");
}

fn assert_cancelled_artifacts_removed(root: &std::path::Path) {
    assert!(!root.join("source.online-migration-target").exists());
    assert!(!root
        .join("source.online-migration-target.migration-journal")
        .exists());
}

#[test]
fn recover_reopens_a_failed_capturing_job_and_commits_it() {
    let root = tempfile::tempdir().expect("root");
    let source = root.path().join("source");
    let source_embedder: DynEmbedder = Box::new(HashEmbedder::new(2));
    let service = MemoryService::open(&source, source_embedder).expect("source");
    service.remember("one", &[], None).expect("fact");
    let slot = Arc::new(LiveGenerationSlot::new(service, "source-model"));
    let factory_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed_calls = Arc::clone(&factory_calls);
    let factory = Arc::new(move |_backend: &str| {
        let call = observed_calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let embedder: DynEmbedder = if call == 0 {
            Box::new(FailAfterWitness {
                calls: std::sync::atomic::AtomicUsize::new(0),
                inner: HashEmbedder::new(3),
            })
        } else {
            Box::new(HashEmbedder::new(3))
        };
        Ok(JobTarget {
            embedder,
            model: "target-model".to_owned(),
        })
    });
    let manager = OnlineMigrationManager::new(Arc::clone(&slot), source, factory).expect("manager");
    manager.start("recoverable", config()).expect("start");
    wait_for(&manager, |status| {
        status.record.phase == JobPhase::Capturing
            && status.record.last_error.is_some()
            && !status.running
    });

    manager.recover().expect("recover");
    wait_for(&manager, |status| {
        status.record.phase == JobPhase::Committed
    });
    assert_eq!(factory_calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    slot.inspect_active(|model, dimension, _| {
        assert_eq!(model, "target-model");
        assert_eq!(dimension, 3);
    })
    .expect("target generation");
}

fn wait_for(
    manager: &OnlineMigrationManager<DynEmbedder>,
    done: impl Fn(&super::manager::MigrationStatus) -> bool,
) {
    let deadline = Instant::now() + FUNCTIONAL_TIMEOUT;
    loop {
        let status = manager.status().expect("status").expect("job");
        if done(&status) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "phase: {:?}",
            status.record.phase
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn config() -> MigrationStartConfig {
    MigrationStartConfig {
        journal_max_bytes: 1024 * 1024,
        catch_up: CatchUpConfig {
            fact_batch: 16,
            replay_batch: 16,
            edge_cap: 16,
        },
        controller: ControllerConfig {
            observation_window: 2,
            pause_budget: FUNCTIONAL_TIMEOUT,
            verification_reserve: Duration::from_millis(10),
        },
    }
}

// ---------------------------------------------------------------------------
// A worker failure is recorded when it can be, and never corrupts the record
// when it cannot.
// ---------------------------------------------------------------------------

fn failed_job_record(root: &std::path::Path) -> super::job_state::JobRecord {
    use super::job_state::JobSpec;
    use crate::mutation::journal::EpochIdentity;
    let identity = EpochIdentity::for_test(
        root.join("source"),
        "source-model",
        "target-model",
        3,
        &format!("sha256:{}", "ab".repeat(32)),
        root.join("destination"),
        "0123456789abcdef0123456789abcdef",
    );
    super::job_state::JobRecord::new(JobSpec {
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
        workspace: root.join("workspace"),
    })
    .expect("record")
}

#[test]
fn a_worker_failure_is_recorded_on_the_job_for_migration_status() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let record = failed_job_record(dir.path());
    let store = super::job_state::JobStore::create(&workspace, &record).expect("create");

    let error = super::manager::capture("the worker died");
    super::manager::persist_worker_error(&store, &error);

    let reloaded = store.load().expect("load");
    assert_eq!(reloaded.last_error, Some(error.to_string()));
}

/// The degraded path: the job record cannot be reached at all. The function
/// must return (it runs on a dying worker thread — a panic there is a lost
/// thread, not a report), and the record must be exactly what it was. What
/// it DOES do here is log — the one channel left — which a unit test cannot
/// assert without a subscriber; the property pinned is that failing to
/// record is itself survivable and non-destructive.
///
/// The workspace is parked and a plain FILE put in its place, so both the
/// load and any staged write fail with "not a directory". (A read-only
/// directory would not do: the suite runs as root in CI containers, and root
/// ignores permission bits.)
#[test]
fn a_worker_failure_that_cannot_be_recorded_does_not_panic_or_corrupt_the_record() {
    let dir = tempfile::tempdir().expect("tempdir");
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let record = failed_job_record(dir.path());
    let store = super::job_state::JobStore::create(&workspace, &record).expect("create");
    let before = std::fs::read(store.path_for_test()).expect("read record");

    let parked = dir.path().join("parked");
    std::fs::rename(&workspace, &parked).expect("park the workspace");
    std::fs::write(&workspace, b"not a directory").expect("occupy the path");
    super::manager::persist_worker_error(&store, &super::manager::capture("the worker died"));
    std::fs::remove_file(&workspace).expect("free the path");
    std::fs::rename(&parked, &workspace).expect("restore the workspace");

    let after = std::fs::read(store.path_for_test()).expect("read record");
    assert_eq!(
        before, after,
        "an unreachable record must come back byte-identical"
    );
    assert_eq!(
        store.load().expect("load").last_error,
        None,
        "nothing was recorded — and nothing was corrupted"
    );
}
