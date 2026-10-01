//! Recovery proofs for the durable extraction state machine.

use super::*;
use crate::embedder::HashEmbedder;
use crate::extract::{ExtractError, ExtractedFact, Extractor};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

struct GenerationMustNotRun {
    calls: AtomicUsize,
}

impl Extractor for GenerationMustNotRun {
    fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, ExtractError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        panic!("a persisted extraction must not be generated again")
    }
}

fn accepted_record() -> JobRecord {
    JobRecord::accepted(
        "a".repeat(64),
        "b".repeat(64),
        PersistedRequest {
            text: "source passage".to_owned(),
            metadata: None,
            backend: Some("outline".to_owned()),
        },
    )
}

#[test]
fn persisted_states_reject_fields_owned_by_another_phase() {
    let mut accepted_with_outcome = accepted_record();
    accepted_with_outcome.outcome = Some(super::super::extraction_job_model::JobOutcome {
        ids: vec![1],
        skipped_over_cap: 0,
    });
    assert!(accepted_with_outcome.validate().is_err());

    let mut committed_with_request = accepted_record();
    committed_with_request.state = ExtractionJobState::Committed;
    committed_with_request.outcome = accepted_with_outcome.outcome;
    assert!(committed_with_request.validate().is_err());
}

#[test]
fn persisted_failure_text_respects_its_utf8_byte_limit() {
    let truncated = truncate_error("é".repeat(4_096));

    assert!(truncated.len() <= 4_096);
    assert!(truncated.ends_with('…'));
    assert!(truncated.is_char_boundary(truncated.len()));
}

#[cfg(unix)]
#[test]
fn a_job_record_swapped_for_a_symlink_is_refused_not_followed() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("create durable job store");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let secret = outside.path().join("secret");
    std::fs::write(&secret, b"do-not-read-me").expect("plant secret");

    let record = accepted_record();
    let store = JobStore::open(directory.path()).expect("open job snapshots");
    store.save(&record).expect("persist record");

    let record_path = directory
        .path()
        .join("extraction-jobs")
        .join(format!("{}.json", record.request_id));
    std::fs::remove_file(&record_path).expect("remove real record");
    symlink(&secret, &record_path).expect("plant symlink record");

    let error = store
        .load(&record.request_id)
        .expect_err("a symlinked job record must be refused, not followed");
    assert!(
        error.to_string().contains("invalid extraction job record"),
        "the refusal must name the invalid record: {error}"
    );
    assert_eq!(
        std::fs::read(&secret).expect("secret must be untouched"),
        b"do-not-read-me",
        "a refused load must never read through the symlink into a file outside the store"
    );
}

/// A hard link is, structurally, an ordinary regular file: `O_NOFOLLOW`
/// cannot tell it apart (#2407). `open_regular_file`'s `nlink > 1` check
/// refuses it on the handle this load already produced, isolated from the
/// symlink case above by keeping every other variable (path, content,
/// request id) identical.
#[cfg(unix)]
#[test]
fn a_job_record_hard_linked_from_outside_is_refused_not_followed() {
    let directory = tempfile::tempdir().expect("create durable job store");

    let record = accepted_record();
    let store = JobStore::open(directory.path()).expect("open job snapshots");
    store.save(&record).expect("persist record");

    let record_path = directory
        .path()
        .join("extraction-jobs")
        .join(format!("{}.json", record.request_id));
    let content = std::fs::read(&record_path).expect("read real record");
    std::fs::remove_file(&record_path).expect("remove real record");
    let alias = directory.path().join("record-alias.json");
    std::fs::write(&alias, &content).expect("plant alias");
    std::fs::hard_link(&alias, &record_path).expect("plant hard link record");

    let error = store
        .load(&record.request_id)
        .expect_err("a hard-linked job record must be refused, not followed");
    assert!(
        error.to_string().contains("invalid extraction job record"),
        "the refusal must name the invalid record: {error}"
    );
    assert_eq!(
        std::fs::read(&alias).expect("alias must be untouched"),
        content,
        "a refused load must never alter the aliased record through the hard link"
    );
}

fn persist_interrupted_job(
    directory: &Path,
    service: &MemoryService<DynEmbedder>,
) -> (String, usize, Option<usize>) {
    let request = PersistedRequest {
        text: "source passage".to_owned(),
        metadata: None,
        backend: None,
    };
    let encoded = serde_json::to_vec(&request).expect("serialize request");
    let input_digest = hex_digest(b"velesdb extraction input v1\0", &encoded);
    let request_id = request_id(None, &encoded).expect("derive request id");
    let extraction = Extraction {
        facts: vec![ExtractedFact {
            text: "Recovered fact is written exactly once.".to_owned(),
            entities: vec!["recovery".to_owned()],
        }],
        ..Extraction::default()
    };
    service
        .store_extraction(&extraction, None)
        .expect("simulate writes completed before the process stopped");
    let facts_before_replay = service.fact_count();
    let edges_before_replay = service.edge_count();
    let record = JobRecord {
        version: RECORD_VERSION,
        request_id: request_id.clone(),
        input_digest,
        state: ExtractionJobState::Running,
        request: Some(request),
        extraction: Some(extraction),
        outcome: None,
        error: None,
    };
    JobStore::open(directory)
        .expect("open job snapshots")
        .save(&record)
        .expect("persist interrupted running job");
    (request_id, facts_before_replay, edges_before_replay)
}

fn wait_for_terminal(jobs: &ExtractionJobs, request_id: &str) -> JobView {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = jobs.status(request_id).expect("read recovered status");
        if status.state.is_terminal() {
            return status;
        }
        assert!(Instant::now() < deadline, "recovered job must finish");
        std::thread::yield_now();
    }
}

fn assert_write_remains_exactly_once(
    service: &MemoryService<DynEmbedder>,
    facts_before_replay: usize,
    edges_before_replay: Option<usize>,
) {
    assert_eq!(service.fact_count(), facts_before_replay);
    assert_eq!(service.edge_count(), edges_before_replay);
    let recalled = service
        .recall("recovered written", 10, None)
        .expect("recall recovered write");
    assert_eq!(
        recalled
            .iter()
            .filter(|memory| memory.content == "Recovered fact is written exactly once.")
            .count(),
        1
    );
}

#[test]
fn recovery_commits_persisted_extraction_without_second_generation() {
    let directory = tempfile::tempdir().expect("create durable job store");
    let embedder: DynEmbedder = Box::new(HashEmbedder::new(crate::DEFAULT_DIMENSION));
    let service =
        MemoryService::open(directory.path(), embedder).expect("open native memory service");
    let (request_id, facts_before_replay, edges_before_replay) =
        persist_interrupted_job(directory.path(), &service);
    let service = Arc::new(LiveGenerationSlot::new(service, "hash"));

    let extractor = Arc::new(GenerationMustNotRun {
        calls: AtomicUsize::new(0),
    });
    let resolver = Arc::new(RwLock::new(ExtractorResolver::unnamed(extractor.clone())));
    let jobs = ExtractionJobs::open(directory.path(), Arc::clone(&service), resolver)
        .expect("recover durable worker");
    let status = wait_for_terminal(&jobs, &request_id);

    assert_eq!(status.state, ExtractionJobState::Committed);
    assert_eq!(status.outcome.expect("committed outcome").ids.len(), 1);
    assert_eq!(extractor.calls.load(Ordering::SeqCst), 0);
    service
        .inspect(|current| {
            assert_write_remains_exactly_once(current, facts_before_replay, edges_before_replay);
        })
        .expect("active generation");
}
