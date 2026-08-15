use std::sync::Arc;
use std::time::Duration;

use super::job_runner::{run_job, JobRunOutcome, JobTarget};
use super::job_state::{JobPhase, JobRecord, JobSpec, JobStore};
use super::LiveGenerationSlot;
use crate::embedding_provenance::{self, EmbeddingProvenance};
use crate::migration::target_embedder_witness;
use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::ControllerConfig;
use crate::mutation::journal::EpochIdentity;
use crate::storage::NativeStore;
use crate::{HashEmbedder, MemoryService};

#[test]
fn a_quiet_job_rebuilds_activates_and_commits_durably() {
    let rig = quiet_job();
    let outcome = run_job(
        &rig.slot,
        &rig.store,
        rig.record,
        JobTarget {
            embedder: HashEmbedder::new(3),
            model: "target-model".to_owned(),
        },
        || false,
    )
    .expect("online migration");

    assert_eq!(outcome, JobRunOutcome::Committed);
    let persisted = rig.store.load().expect("persisted job");
    assert_eq!(persisted.phase, JobPhase::Committed);
    assert!(persisted.progress.measured_cutover.is_some());
    rig.slot
        .inspect_active(|model, dimension, service| {
            assert_eq!(model, "target-model");
            assert_eq!(dimension, 3);
            assert_eq!(service.fact_count(), 2);
            assert_eq!(service.edge_count(), Some(1));
        })
        .expect("active target");
}

struct QuietJobRig {
    _root: tempfile::TempDir,
    slot: Arc<LiveGenerationSlot<HashEmbedder>>,
    store: JobStore,
    record: JobRecord,
}

fn quiet_job() -> QuietJobRig {
    let root = tempfile::tempdir().expect("root");
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let workspace = root.path().join("destination.migration-journal");
    let control = root.path().join("control");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::create_dir_all(&control).expect("control");
    let service = populated_service(&source);
    let slot = Arc::new(LiveGenerationSlot::new(service, "source-model"));
    drop(NativeStore::open(&destination, 3).expect("destination"));
    embedding_provenance::write(&destination, &EmbeddingProvenance::new("target-model", 3))
        .expect("provenance");
    let record = job_record(source, destination, workspace);
    let store = JobStore::create(&control, &record).expect("job store");
    QuietJobRig {
        _root: root,
        slot,
        store,
        record,
    }
}

fn populated_service(path: &std::path::Path) -> MemoryService<HashEmbedder, NativeStore> {
    let service = MemoryService::open(path, HashEmbedder::new(2)).expect("source");
    let first = service
        .remember("first durable fact", &[], None)
        .expect("first");
    let second = service
        .remember("second durable fact", &[], None)
        .expect("second");
    service.relate(first, second, "supports").expect("edge");
    service
}

fn job_record(
    source: std::path::PathBuf,
    destination: std::path::PathBuf,
    workspace: std::path::PathBuf,
) -> JobRecord {
    let target = HashEmbedder::new(3);
    let identity = EpochIdentity::for_test(
        source,
        "source-model",
        "target-model",
        3,
        &target_embedder_witness(&target).expect("witness"),
        destination,
        "00112233445566778899aabbccddeeff",
    );
    JobRecord::new(JobSpec {
        identity,
        target_backend: "hash".to_owned(),
        journal_max_bytes: 1024 * 1024,
        catch_up: CatchUpConfig {
            fact_batch: 16,
            replay_batch: 16,
            edge_cap: 16,
        },
        controller: ControllerConfig {
            observation_window: 2,
            // Functional fixture; deadline refusal has dedicated controller tests.
            pause_budget: Duration::from_secs(30),
            verification_reserve: Duration::from_millis(10),
        },
        workspace,
    })
    .expect("record")
}
