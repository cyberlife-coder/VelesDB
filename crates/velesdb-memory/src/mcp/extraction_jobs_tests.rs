//! Recovery proofs for the durable extraction state machine.

use super::*;
use crate::embedder::HashEmbedder;
use crate::extract::{ExtractError, ExtractedFact, Extractor};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
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

/// Opens a fresh job store under `directory`, persists one accepted record,
/// and returns its on-disk path — the shared setup for the symlink/hard-link/
/// permission-denied tests below, which each go on to tamper with that record
/// (most replace or chmod the exact path; one chmods its containing directory
/// instead) to prove either a refusal or, for the hard-link case, acceptance.
#[cfg(unix)]
fn seed_saved_record(directory: &tempfile::TempDir) -> (JobStore, JobRecord, PathBuf) {
    let record = accepted_record();
    let store = JobStore::open(directory.path()).expect("open job snapshots");
    store.save(&record).expect("persist record");
    let record_path = directory
        .path()
        .join("extraction-jobs")
        .join(format!("{}.json", record.request_id));
    (store, record, record_path)
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

    let (store, record, record_path) = seed_saved_record(&directory);
    // The secret holds a VALID job record under the same request_id: a
    // load that merely followed the symlink would parse it fine and
    // return it as the real one. Planting unparsable bytes instead would
    // make this test pass for the wrong reason — refused only because the
    // content fails to decode, never exercising whether the symlink
    // itself is followed (the migration lock's own
    // `a_lock_file_swapped_for_a_symlink_is_refused_not_followed` makes
    // the same point).
    let mut leaked = record.clone();
    leaked.request = Some(PersistedRequest {
        text: "LEAKED-SECRET".to_owned(),
        metadata: None,
        backend: None,
    });
    let secret = outside.path().join("secret.json");
    std::fs::write(
        &secret,
        serde_json::to_vec(&leaked).expect("serialize secret"),
    )
    .expect("plant secret");

    std::fs::remove_file(&record_path).expect("remove real record");
    symlink(&secret, &record_path).expect("plant symlink record");

    let error = store
        .load(&record.request_id)
        .expect_err("a symlinked job record must be refused, not followed");
    assert!(
        error.to_string().contains("invalid extraction job record"),
        "the refusal must name the invalid record: {error}"
    );
}

/// #2409 round 23: the test above only swaps the symlink in BEFORE calling
/// `load` — the pre-fix `symlink_metadata`-then-reopen-by-name shape
/// refuses that static case too, since its own check already sees the
/// symlink; it never exercises the WINDOW between a check and a later
/// reopen by name, which only a continuously racing swap can reach.
/// Round 23's own mutation proof ran against a bare `open` with no guard
/// at all, not the real pre-fix shape. Round 24 reconstructed the real
/// shape (from this branch's merge-base) and found the race, swapped via
/// `remove` then `symlink`/`write`, unreliable even at `ITERATIONS =
/// 200_000`: the path is briefly ABSENT between the two syscalls on each
/// side of the swap, wasting most iterations. Round 25 swaps the path
/// atomically instead — `rename` a staged symlink or a staged real file
/// into place, so the path is always either one or the other, never
/// absent, and brought `ITERATIONS` down to `20_000`. Round 26 found
/// that fix still unreliable under the DEFAULT parallel test harness
/// (`cargo test`, as opposed to `--test-threads=1`): other tests
/// contending for CPU change how far the reader and the writer get
/// relative to each other, and a fixed `0..ITERATIONS` reader loop can
/// finish long before the writer and under-sample the race. The reader
/// below now runs for as long as the writer thread is alive instead,
/// which is reliable under both. The OLD fixed-count reader (`for _ in
/// 0..ITERATIONS`) only leaked 26 to 47 reads of the secret's own
/// marker per run against the real pre-fix shape, isolated — the
/// under-sampling this round fixes. The reader below, run against the
/// same pre-fix shape: isolated (`--test-threads=1`), 5 repeats leaked
/// 1,822 to 2,063 times each; under the default parallel harness, 3
/// repeats of the FULL suite leaked 2,190 to 2,449 times each. The
/// fixed code passes cleanly under both conditions (round 27).
#[cfg(unix)]
#[test]
fn a_job_record_is_never_read_through_a_racing_symlink_swap() {
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ITERATIONS: usize = 20_000;

    let directory = tempfile::tempdir().expect("create durable job store");
    let outside = tempfile::tempdir().expect("outside tempdir");

    let (store, record, record_path) = seed_saved_record(&directory);
    let real_body = std::fs::read(&record_path).expect("read real record");
    let staging_symlink = record_path.with_extension("staging-symlink");
    let staging_real = record_path.with_extension("staging-real");

    let mut leaked = record.clone();
    leaked.request = Some(PersistedRequest {
        text: "LEAKED-SECRET".to_owned(),
        metadata: None,
        backend: None,
    });
    let secret_path = outside.path().join("secret.json");
    std::fs::write(
        &secret_path,
        serde_json::to_vec(&leaked).expect("serialize secret"),
    )
    .expect("plant secret");

    let leaked_count = AtomicUsize::new(0);

    std::thread::scope(|scope| {
        let writer = scope.spawn(|| {
            for _ in 0..ITERATIONS {
                symlink(&secret_path, &staging_symlink).expect("stage symlink");
                std::fs::rename(&staging_symlink, &record_path)
                    .expect("atomically swap in the symlink");
                std::fs::write(&staging_real, &real_body).expect("stage real record");
                std::fs::rename(&staging_real, &record_path)
                    .expect("atomically swap in the real record");
            }
        });

        // Read for as long as the writer is still swapping, not a matching
        // fixed count: under the default parallel test harness (as opposed
        // to `--test-threads=1`), other tests contend for CPU and the two
        // threads here no longer make comparable progress per iteration,
        // so a fixed `0..ITERATIONS` reader loop can finish long before the
        // writer and under-sample the race (#2409 round 26).
        while !writer.is_finished() {
            if let Ok(Some(loaded)) = store.load(&record.request_id) {
                if loaded
                    .request
                    .as_ref()
                    .is_some_and(|request| request.text == "LEAKED-SECRET")
                {
                    leaked_count.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        writer.join().expect("writer must not panic");
    });

    assert_eq!(
        leaked_count.load(Ordering::Relaxed),
        0,
        "a racing symlink swap must never be read through to the caller"
    );
}

/// A genuine I/O failure opening a job record (here, permission denied) must
/// keep its own detail, not be relabeled with `NotARegularFile`'s "invalid
/// extraction job record" wording — that wording is for a deliberate
/// refusal (a symlink swap), not an unrelated I/O error the same
/// `open_regular_file_allow_hard_links` call can also raise.
#[cfg(unix)]
#[test]
fn a_permission_denied_job_record_keeps_its_io_detail_not_a_refusal_label() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("create durable job store");
    let (store, record, record_path) = seed_saved_record(&directory);
    std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 000");

    if std::fs::File::open(&record_path).is_ok() {
        eprintln!("test: skipped, permission bits are not enforced here");
        std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o644))
            .expect("restore");
        return;
    }

    let error = store
        .load(&record.request_id)
        .expect_err("a permission-denied record must surface as an error");
    let message = error.to_string();
    assert!(
        message.contains("Permission denied") || message.contains("permission denied"),
        "an I/O failure opening the record must keep its real detail, not the refusal's \
         \"invalid extraction job record\" wording meant for a deliberately wrong file type: \
         {message}"
    );
    assert!(
        !message.contains("invalid extraction job record"),
        "a genuine I/O failure must not be relabeled as the NotARegularFile refusal: {message}"
    );
    assert!(
        !message.contains("migration capture error"),
        "an unrelated I/O failure must not carry the online-migration observer's own wording \
         (#2409 round 13): {message}"
    );

    std::fs::set_permissions(&record_path, std::fs::Permissions::from_mode(0o644))
        .expect("restore permissions");
}

/// `read_record_bytes`'s `path_exists` check (ahead of the open) raises a
/// genuine stat failure as `MemoryError::MigrationCapture`, worded for the
/// online-migration observer, not this unrelated caller — it must still
/// surface its own I/O detail, not that wording (#2409 round 12).
#[cfg(unix)]
#[test]
fn a_record_directory_lookup_failure_keeps_its_io_detail_not_the_migration_capture_prefix() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().expect("create durable job store");
    let (store, record, record_path) = seed_saved_record(&directory);
    let jobs_directory = directory.path().join("extraction-jobs");
    std::fs::set_permissions(&jobs_directory, std::fs::Permissions::from_mode(0o000))
        .expect("chmod 000 on the job directory");

    if std::fs::symlink_metadata(&record_path).is_ok() {
        eprintln!("test: skipped, directory permission bits are not enforced here");
        std::fs::set_permissions(&jobs_directory, std::fs::Permissions::from_mode(0o755))
            .expect("restore");
        return;
    }

    let error = store
        .load(&record.request_id)
        .expect_err("a directory lookup failure must surface as an error");
    let message = error.to_string();
    assert!(
        message.contains("Permission denied") || message.contains("permission denied"),
        "a stat failure on the record's directory must keep its real detail: {message}"
    );
    assert!(
        !message.contains("migration capture error"),
        "an unrelated I/O failure must not carry the online-migration observer's own wording: \
         {message}"
    );

    std::fs::set_permissions(&jobs_directory, std::fs::Permissions::from_mode(0o755))
        .expect("restore permissions");
}

/// A hard link is, structurally, an ordinary regular file: `O_NOFOLLOW`
/// cannot tell it apart (#2407). Unlike a journal file this store reopens to
/// write into across its lifetime, a job record is only ever replaced
/// wholesale by `save`'s rename — so a hard-link based backup of the store
/// (`cp -al`, `rsync --link-dest`) leaves `nlink > 1` on an ordinary,
/// untampered record, and refusing it would break this store's own startup
/// scan (`pending`, which treats any refused record as fatal) after a
/// legitimate backup restore (#2409 round 7). Proven by mutation: reverting
/// `read_record_bytes` to `open_regular_file` makes this test fail with
/// "invalid extraction job record".
#[cfg(unix)]
#[test]
fn a_job_record_hard_linked_from_outside_is_loaded_not_refused() {
    let directory = tempfile::tempdir().expect("create durable job store");
    let outside = tempfile::tempdir().expect("outside tempdir");
    let (store, record, record_path) = seed_saved_record(&directory);
    let content = std::fs::read(&record_path).expect("read real record");
    std::fs::remove_file(&record_path).expect("remove real record");
    let alias = outside.path().join("record-alias.json");
    std::fs::write(&alias, &content).expect("plant alias");
    std::fs::hard_link(&alias, &record_path).expect("plant hard link record");

    let loaded = store
        .load(&record.request_id)
        .expect("a hard-linked job record must be loaded, not refused")
        .expect("record must be found");
    assert_eq!(loaded.request_id, record.request_id);

    let pending = store
        .pending()
        .expect("a startup scan must not fail on a hard-linked record");
    assert_eq!(
        pending,
        vec![record.request_id],
        "the startup scan must still see the hard-linked record as pending"
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
