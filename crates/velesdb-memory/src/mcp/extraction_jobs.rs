//! Durable state machine behind the asynchronous `remember_extracted` tool.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use parking_lot::{Mutex, RwLock};
use sha2::{Digest, Sha256};

#[cfg(test)]
use super::extraction_job_model::RECORD_VERSION;
pub(super) use super::extraction_job_model::{
    valid_digest, ExtractionJobState, JobError, JobReceipt, JobRecord, JobView, PersistedRequest,
};
use super::extraction_job_store::{storage_error, JobStore};
use super::extractor_resolver::ExtractorResolver;
use crate::embedder::DynEmbedder;
use crate::extract::Extraction;
use crate::model::RememberedExtraction;
use crate::service::{LiveGenerationSlot, MemoryService, Metadata};

#[derive(Default)]
struct RuntimeState {
    pending: usize,
    queued: HashSet<String>,
}

struct PreparedSubmission {
    request: PersistedRequest,
    input_digest: String,
    request_id: String,
}

struct Shared {
    service: Arc<LiveGenerationSlot<DynEmbedder>>,
    extractors: Arc<RwLock<ExtractorResolver>>,
    store: JobStore,
    runtime: Mutex<RuntimeState>,
    closing: Arc<AtomicBool>,
}

enum Command {
    Run(String),
    Stop,
}

struct WorkerGuard {
    closing: Arc<AtomicBool>,
    tx: mpsc::Sender<Command>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.closing.store(true, Ordering::Release);
        let _ = self.tx.send(Command::Stop);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Cloneable front-end to one serial, durable extraction worker.
#[derive(Clone)]
pub(super) struct ExtractionJobs {
    shared: Arc<Shared>,
    tx: mpsc::Sender<Command>,
    _guard: Arc<WorkerGuard>,
}

impl ExtractionJobs {
    pub(super) fn open(
        root: &Path,
        service: Arc<LiveGenerationSlot<DynEmbedder>>,
        extractors: Arc<RwLock<ExtractorResolver>>,
    ) -> Result<Self, JobError> {
        let store = JobStore::open(root)?;
        let pending = store.pending()?;
        if pending.len() > crate::limits::MAX_EXTRACTION_JOBS {
            return Err(JobError::Storage(format!(
                "{} non-terminal extraction jobs exceed the limit of {}",
                pending.len(),
                crate::limits::MAX_EXTRACTION_JOBS
            )));
        }
        let closing = Arc::new(AtomicBool::new(false));
        let runtime = RuntimeState {
            pending: pending.len(),
            queued: pending.iter().cloned().collect(),
        };
        let shared = Arc::new(Shared {
            service,
            extractors,
            store,
            runtime: Mutex::new(runtime),
            closing: Arc::clone(&closing),
        });
        let (tx, rx) = mpsc::channel();
        let worker_shared = Arc::clone(&shared);
        let join = std::thread::Builder::new()
            .name("velesdb-extraction-jobs".to_owned())
            .spawn(move || worker_loop(&worker_shared, &rx))
            .map_err(|error| JobError::Storage(format!("cannot spawn worker: {error}")))?;
        for request_id in pending {
            tx.send(Command::Run(request_id))
                .map_err(|_| JobError::Storage("worker stopped during recovery".to_owned()))?;
        }
        let guard = Arc::new(WorkerGuard {
            closing,
            tx: tx.clone(),
            join: Some(join),
        });
        Ok(Self {
            shared,
            tx,
            _guard: guard,
        })
    }

    pub(super) fn submit(
        &self,
        text: &str,
        metadata: Option<Metadata>,
        requested_backend: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> Result<JobReceipt, JobError> {
        let prepared = prepare_submission(text, metadata, requested_backend, idempotency_key)?;
        let mut runtime = self.shared.runtime.lock();
        if let Some(receipt) = self.reused_receipt(&prepared, &mut runtime)? {
            return Ok(receipt);
        }
        self.accept_new(prepared, requested_backend, &mut runtime)
    }

    fn reused_receipt(
        &self,
        prepared: &PreparedSubmission,
        runtime: &mut RuntimeState,
    ) -> Result<Option<JobReceipt>, JobError> {
        let Some(existing) = self.shared.store.load(&prepared.request_id)? else {
            return Ok(None);
        };
        if existing.input_digest != prepared.input_digest {
            return Err(JobError::Conflict);
        }
        enqueue_if_needed(&self.tx, runtime, &prepared.request_id, existing.state)?;
        Ok(Some(JobReceipt {
            request_id: prepared.request_id.clone(),
            state: existing.state,
            reused: true,
        }))
    }

    fn accept_new(
        &self,
        mut prepared: PreparedSubmission,
        requested_backend: Option<&str>,
        runtime: &mut RuntimeState,
    ) -> Result<JobReceipt, JobError> {
        prepared.request.backend = self.resolve_backend(requested_backend)?;
        if runtime.pending >= crate::limits::MAX_EXTRACTION_JOBS {
            return Err(JobError::AtCapacity);
        }
        stamp_acceptance_date(&mut prepared.request.metadata);
        let record = JobRecord::accepted(
            prepared.request_id.clone(),
            prepared.input_digest,
            prepared.request,
        );
        self.shared.store.save(&record)?;
        runtime.pending += 1;
        enqueue_if_needed(
            &self.tx,
            runtime,
            &prepared.request_id,
            ExtractionJobState::Accepted,
        )?;
        Ok(JobReceipt {
            request_id: prepared.request_id,
            state: ExtractionJobState::Accepted,
            reused: false,
        })
    }

    fn resolve_backend(&self, requested: Option<&str>) -> Result<Option<String>, JobError> {
        self.shared
            .extractors
            .read()
            .resolve_for_job(requested)
            .map(|resolved| resolved.backend)
            .map_err(|error| match error {
                super::extractor_resolver::ExtractorResolveError::DefaultNotConfigured => {
                    JobError::BackendNotConfigured
                }
                super::extractor_resolver::ExtractorResolveError::InvalidRequest(message) => {
                    JobError::Invalid(message)
                }
            })
    }

    pub(super) fn status(&self, request_id: &str) -> Result<JobView, JobError> {
        if !valid_digest(request_id) {
            return Err(JobError::Invalid(
                "request_id must be 64 lowercase hexadecimal characters".to_owned(),
            ));
        }
        let mut runtime = self.shared.runtime.lock();
        let record = self
            .shared
            .store
            .load(request_id)?
            .ok_or_else(|| JobError::NotFound(request_id.to_owned()))?;
        enqueue_if_needed(&self.tx, &mut runtime, request_id, record.state)?;
        Ok(JobView {
            request_id: record.request_id,
            state: record.state,
            outcome: record.outcome,
            error: record.error,
        })
    }
}

#[derive(serde::Serialize)]
struct InputFingerprint<'a> {
    text: &'a str,
    metadata: &'a Option<Metadata>,
    extractor: Option<&'a str>,
}

fn prepare_submission(
    text: &str,
    metadata: Option<Metadata>,
    requested_backend: Option<&str>,
    idempotency_key: Option<&str>,
) -> Result<PreparedSubmission, JobError> {
    let request = normalized_request(text, metadata)?;
    let fingerprint = InputFingerprint {
        text: &request.text,
        metadata: &request.metadata,
        extractor: requested_backend,
    };
    let encoded = serde_json::to_vec(&fingerprint).map_err(storage_error)?;
    Ok(PreparedSubmission {
        input_digest: hex_digest(b"velesdb extraction input v1\0", &encoded),
        request_id: request_id(idempotency_key, &encoded)?,
        request,
    })
}

fn normalized_request(
    text: &str,
    metadata: Option<Metadata>,
) -> Result<PersistedRequest, JobError> {
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err(JobError::Invalid("text must not be empty".to_owned()));
    }
    validate_metadata(metadata.as_ref())?;
    Ok(PersistedRequest {
        text,
        metadata,
        backend: None,
    })
}

fn validate_metadata(metadata: Option<&Metadata>) -> Result<(), JobError> {
    let Some(metadata) = metadata else {
        return Ok(());
    };
    if let Some(key) = metadata
        .keys()
        .find(|key| crate::storage::is_reserved_key(key))
    {
        return Err(JobError::Invalid(format!(
            "metadata key '{key}' is reserved"
        )));
    }
    let bytes = crate::limits::metadata_bytes(metadata);
    if bytes > crate::limits::MAX_METADATA_BYTES {
        return Err(JobError::Invalid(format!(
            "metadata of {bytes} bytes exceeds the cap of {} bytes",
            crate::limits::MAX_METADATA_BYTES
        )));
    }
    Ok(())
}

fn stamp_acceptance_date(metadata: &mut Option<Metadata>) {
    if metadata
        .as_ref()
        .is_some_and(|value| value.contains_key(crate::storage::AUTO_DATE_FIELD))
    {
        return;
    }
    let Some(today) = crate::clock::today_ymd() else {
        return;
    };
    metadata
        .get_or_insert_with(Metadata::new)
        .insert(crate::storage::AUTO_DATE_FIELD.to_owned(), today.into());
}

fn request_id(key: Option<&str>, encoded: &[u8]) -> Result<String, JobError> {
    let Some(key) = key else {
        return Ok(hex_digest(b"velesdb extraction request v1\0", encoded));
    };
    if key.trim().is_empty() {
        return Err(JobError::Invalid(
            "idempotency_key must not be empty".to_owned(),
        ));
    }
    if key.len() > crate::limits::MAX_IDEMPOTENCY_KEY_BYTES {
        return Err(JobError::Invalid(format!(
            "idempotency_key exceeds {} bytes",
            crate::limits::MAX_IDEMPOTENCY_KEY_BYTES
        )));
    }
    Ok(hex_digest(
        b"velesdb extraction idempotency key v1\0",
        key.as_bytes(),
    ))
}

fn hex_digest(domain: &[u8], body: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(body);
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn enqueue_if_needed(
    tx: &mpsc::Sender<Command>,
    runtime: &mut RuntimeState,
    request_id: &str,
    state: ExtractionJobState,
) -> Result<(), JobError> {
    if state.is_terminal() || !runtime.queued.insert(request_id.to_owned()) {
        return Ok(());
    }
    if tx.send(Command::Run(request_id.to_owned())).is_err() {
        runtime.queued.remove(request_id);
        return Err(JobError::Storage(
            "extraction worker is unavailable".to_owned(),
        ));
    }
    Ok(())
}

fn worker_loop(shared: &Arc<Shared>, rx: &mpsc::Receiver<Command>) {
    while let Ok(command) = rx.recv() {
        if shared.closing.load(Ordering::Acquire) {
            break;
        }
        let Command::Run(request_id) = command else {
            break;
        };
        let terminal = process_job(shared, &request_id);
        let mut runtime = shared.runtime.lock();
        runtime.queued.remove(&request_id);
        if terminal {
            runtime.pending = runtime.pending.saturating_sub(1);
        }
    }
}

fn process_job(shared: &Shared, request_id: &str) -> bool {
    let Ok(Some(mut record)) = shared.store.load(request_id) else {
        return false;
    };
    if record.state.is_terminal() {
        return true;
    }
    record.state = ExtractionJobState::Running;
    if let Err(error) = shared.store.save(&record) {
        tracing::error!(%request_id, %error, "cannot persist extraction job transition");
        return false;
    }
    match execute_job(shared, &mut record) {
        Ok(outcome) => finish_committed(shared, record, outcome),
        Err(error) => finish_failed(shared, record, error),
    }
}

fn execute_job(shared: &Shared, record: &mut JobRecord) -> Result<RememberedExtraction, String> {
    ensure_extraction(shared, record)?;
    let Some(extraction) = record.extraction.as_ref() else {
        return Err("job extraction is missing".to_owned());
    };
    let Some(request) = record.request.as_ref() else {
        return Err("job request is missing".to_owned());
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        shared
            .service
            .run(|current| current.store_extraction(extraction, request.metadata.as_ref()))
    }));
    match result {
        Ok(Ok(outcome)) => Ok(outcome),
        Ok(Err(error)) => Err(error.to_string()),
        Err(payload) => Err(panic_text(payload.as_ref())),
    }
}

fn ensure_extraction(shared: &Shared, record: &mut JobRecord) -> Result<(), String> {
    if record.extraction.is_some() {
        return Ok(());
    }
    record.extraction = Some(generate(shared, record)?);
    shared.store.save(record).map_err(|error| error.to_string())
}

fn generate(shared: &Shared, record: &JobRecord) -> Result<Extraction, String> {
    let request = record
        .request
        .as_ref()
        .ok_or_else(|| "job request is missing".to_owned())?;
    let resolved = shared
        .extractors
        .read()
        .resolve_for_job(request.backend.as_deref())
        .map_err(extractor_error_text)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        MemoryService::<DynEmbedder>::extract_passage(&request.text, &resolved.extractor)
    }));
    match result {
        Ok(Ok(extraction)) => Ok(extraction),
        Ok(Err(error)) => Err(error.to_string()),
        Err(payload) => Err(panic_text(payload.as_ref())),
    }
}

fn extractor_error_text(error: super::extractor_resolver::ExtractorResolveError) -> String {
    match error {
        super::extractor_resolver::ExtractorResolveError::DefaultNotConfigured => {
            "extraction backend is no longer configured".to_owned()
        }
        super::extractor_resolver::ExtractorResolveError::InvalidRequest(message) => message,
    }
}

fn finish_committed(shared: &Shared, mut record: JobRecord, outcome: RememberedExtraction) -> bool {
    record.state = ExtractionJobState::Committed;
    record.request = None;
    record.extraction = None;
    record.outcome = Some(outcome.into());
    record.error = None;
    persist_terminal(shared, &record)
}

fn finish_failed(shared: &Shared, mut record: JobRecord, error: String) -> bool {
    record.state = ExtractionJobState::Failed;
    record.request = None;
    record.extraction = None;
    record.outcome = None;
    record.error = Some(truncate_error(error));
    persist_terminal(shared, &record)
}

fn persist_terminal(shared: &Shared, record: &JobRecord) -> bool {
    match shared.store.save(record) {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(request_id = %record.request_id, %error, "cannot persist terminal extraction job");
            false
        }
    }
}

fn truncate_error(mut error: String) -> String {
    const MAX_ERROR_BYTES: usize = 4_096;
    const ELLIPSIS: &str = "…";
    if error.len() <= MAX_ERROR_BYTES {
        return error;
    }
    let mut end = MAX_ERROR_BYTES - ELLIPSIS.len();
    while !error.is_char_boundary(end) {
        end -= 1;
    }
    error.truncate(end);
    error.push_str(ELLIPSIS);
    error
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        return format!("extraction worker panicked: {message}");
    }
    if let Some(message) = payload.downcast_ref::<String>() {
        return format!("extraction worker panicked: {message}");
    }
    "extraction worker panicked".to_owned()
}

#[cfg(test)]
#[path = "extraction_jobs_tests.rs"]
mod tests;
