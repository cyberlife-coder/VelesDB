use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::mutation::catchup::CatchUpConfig;
use crate::mutation::controller::{ControllerConfig, ConvergenceObservation};
use crate::mutation::journal::EpochIdentity;
use crate::MemoryError;

const JOB_VERSION: u32 = 1;
const JOB_FILE: &str = "online-migration-job.json";
const STAGING_FILE: &str = "online-migration-job.json.tmp";
const MAX_JOB_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct JobSpec {
    pub(crate) identity: EpochIdentity,
    pub(crate) target_backend: String,
    pub(crate) journal_max_bytes: u64,
    pub(crate) catch_up: CatchUpConfig,
    pub(crate) controller: ControllerConfig,
    pub(crate) workspace: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum JobPhase {
    Prepared,
    Capturing,
    BaseCopied,
    CatchingUp,
    NonConverging,
    CutoverReady,
    Quiescing,
    Activated,
    Committed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct JobProgress {
    pub(crate) base_facts: u64,
    pub(crate) base_edge_sets: u64,
    pub(crate) base_batches: u64,
    pub(crate) input_watermark: u64,
    pub(crate) output_watermark: u64,
    pub(crate) distinct_dirty_facts: u64,
    pub(crate) distinct_edge_sources: u64,
    pub(crate) pending_journal_bytes: u64,
    pub(crate) estimated_pause: Option<std::time::Duration>,
    pub(crate) measured_cutover: Option<std::time::Duration>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct JobRecord {
    version: u32,
    pub(crate) spec: JobSpec,
    pub(crate) phase: JobPhase,
    #[serde(default)]
    pub(crate) progress: JobProgress,
    #[serde(default)]
    pub(crate) cancellation_requested: bool,
    pub(crate) recovery_action: Option<String>,
    pub(crate) last_error: Option<String>,
}

impl JobRecord {
    pub(crate) fn new(spec: JobSpec) -> Result<Self, MemoryError> {
        let record = Self {
            version: JOB_VERSION,
            spec,
            phase: JobPhase::Prepared,
            progress: JobProgress::default(),
            cancellation_requested: false,
            recovery_action: None,
            last_error: None,
        };
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn transition(&mut self, next: JobPhase) -> Result<(), MemoryError> {
        if !allowed_transition(self.phase, next) {
            return Err(capture(format!(
                "unsafe online migration transition {:?} -> {next:?}",
                self.phase
            )));
        }
        self.phase = next;
        self.last_error = None;
        Ok(())
    }

    pub(crate) fn record_base_copy(
        &mut self,
        progress: crate::mutation::catchup::BaseCopyProgress,
    ) {
        self.progress.base_facts = progress.facts;
        self.progress.base_edge_sets = progress.edge_sets;
        self.progress.base_batches = progress.batches;
        self.progress.input_watermark = progress.start_watermark;
    }

    pub(crate) fn record_observation(&mut self, observation: ConvergenceObservation) {
        let metrics = observation.metrics;
        self.progress.input_watermark = metrics.input_watermark;
        self.progress.output_watermark = metrics.output_watermark;
        self.progress.distinct_dirty_facts = metrics.distinct_dirty_facts;
        self.progress.distinct_edge_sources = metrics.distinct_edge_sources;
        self.progress.pending_journal_bytes = metrics.pending_journal_bytes;
        self.progress.estimated_pause = observation.estimated_pause;
    }

    pub(crate) fn request_cancellation(&mut self) {
        self.cancellation_requested = true;
    }

    pub(crate) fn fail(&mut self, error: impl Into<String>) {
        self.last_error = Some(error.into());
    }

    fn validate(&self) -> Result<(), MemoryError> {
        if self.version != JOB_VERSION {
            return Err(capture(format!(
                "unsupported online migration job version {}",
                self.version
            )));
        }
        self.spec.identity.validate()?;
        if self.spec.target_backend.trim().is_empty() || self.spec.journal_max_bytes == 0 {
            return Err(capture("online migration job configuration is incomplete"));
        }
        self.spec.catch_up.validated()?;
        self.spec.controller.validate()?;
        validate_terminal_state(self)
    }
}

#[derive(Clone)]
pub(crate) struct JobStore {
    workspace: PathBuf,
    path: PathBuf,
    access: std::sync::Arc<parking_lot::Mutex<()>>,
}

impl JobStore {
    pub(crate) fn create(workspace: &Path, record: &JobRecord) -> Result<Self, MemoryError> {
        validate_workspace(workspace)?;
        record.validate()?;
        let store = Self {
            workspace: workspace.to_owned(),
            path: workspace.join(JOB_FILE),
            access: std::sync::Arc::new(parking_lot::Mutex::new(())),
        };
        store.recover_staging()?;
        ensure_replaceable(&store)?;
        store.save(record)?;
        Ok(store)
    }

    pub(crate) fn try_open(workspace: &Path) -> Result<Option<Self>, MemoryError> {
        validate_workspace(workspace)?;
        let store = Self {
            workspace: workspace.to_owned(),
            path: workspace.join(JOB_FILE),
            access: std::sync::Arc::new(parking_lot::Mutex::new(())),
        };
        store.recover_staging()?;
        if !path_exists(&store.path)? {
            return Ok(None);
        }
        store.load()?;
        Ok(Some(store))
    }

    pub(crate) fn open(workspace: &Path) -> Result<Self, MemoryError> {
        validate_workspace(workspace)?;
        let store = Self {
            workspace: workspace.to_owned(),
            path: workspace.join(JOB_FILE),
            access: std::sync::Arc::new(parking_lot::Mutex::new(())),
        };
        store.recover_staging()?;
        store.load()?;
        Ok(store)
    }

    pub(crate) fn load(&self) -> Result<JobRecord, MemoryError> {
        let _access = self.access.lock();
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> Result<JobRecord, MemoryError> {
        let bytes = read_limited(&self.path)?;
        let record: JobRecord = serde_json::from_slice(&bytes)
            .map_err(|err| capture(format!("invalid online migration job state: {err}")))?;
        record.validate()?;
        Ok(record)
    }

    pub(crate) fn save(&self, record: &JobRecord) -> Result<(), MemoryError> {
        let _access = self.access.lock();
        record.validate()?;
        self.recover_staging()?;
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|err| capture(format!("cannot encode online migration job: {err}")))?;
        if bytes.len() as u64 > MAX_JOB_BYTES {
            return Err(capture("online migration job exceeds 64 KiB safety limit"));
        }
        let staging = self.workspace.join(STAGING_FILE);
        write_synced(&staging, &bytes)?;
        promote(&staging, &self.path)
            .map_err(|err| capture(format!("cannot publish online migration job: {err}")))?;
        sync_directory(&self.workspace)
            .map_err(|err| capture(format!("cannot sync online migration job: {err}")))
    }

    fn recover_staging(&self) -> Result<(), MemoryError> {
        let staging = self.workspace.join(STAGING_FILE);
        if !path_exists(&staging)? {
            return Ok(());
        }
        validate_regular_file(&staging)?;
        std::fs::remove_file(&staging)
            .map_err(|err| capture(format!("cannot remove job staging file: {err}")))
    }
}

fn ensure_replaceable(store: &JobStore) -> Result<(), MemoryError> {
    if !path_exists(&store.path)? {
        return Ok(());
    }
    let previous = store.load()?;
    if matches!(previous.phase, JobPhase::Committed | JobPhase::Cancelled) {
        return Ok(());
    }
    Err(capture("an online migration job already exists"))
}

fn allowed_transition(current: JobPhase, next: JobPhase) -> bool {
    if current == next {
        return true;
    }
    matches!(
        (current, next),
        (
            JobPhase::Prepared,
            JobPhase::Capturing | JobPhase::Cancelled
        ) | (
            JobPhase::Capturing,
            JobPhase::BaseCopied | JobPhase::Cancelled
        ) | (
            JobPhase::BaseCopied
                | JobPhase::NonConverging
                | JobPhase::CutoverReady
                | JobPhase::Quiescing,
            JobPhase::CatchingUp
        ) | (
            JobPhase::BaseCopied
                | JobPhase::CatchingUp
                | JobPhase::NonConverging
                | JobPhase::CutoverReady,
            JobPhase::Cancelled
        ) | (
            JobPhase::CatchingUp,
            JobPhase::CutoverReady | JobPhase::NonConverging
        ) | (
            JobPhase::CutoverReady,
            JobPhase::NonConverging | JobPhase::Quiescing
        ) | (JobPhase::Quiescing, JobPhase::Activated)
            | (JobPhase::Activated, JobPhase::Committed)
    )
}

fn validate_terminal_state(record: &JobRecord) -> Result<(), MemoryError> {
    let terminal = matches!(record.phase, JobPhase::Committed | JobPhase::Cancelled);
    if terminal && record.recovery_action.is_some() {
        return Err(capture("terminal online migration job requests recovery"));
    }
    Ok(())
}

fn read_limited(path: &Path) -> Result<Vec<u8>, MemoryError> {
    validate_regular_file(path)?;
    let mut file = File::open(path)
        .map_err(|err| capture(format!("cannot open online migration job: {err}")))?;
    let length = file
        .metadata()
        .map_err(|err| capture(format!("cannot size online migration job: {err}")))?
        .len();
    if length > MAX_JOB_BYTES {
        return Err(capture("online migration job exceeds 64 KiB safety limit"));
    }
    let capacity = usize::try_from(length)
        .map_err(|_| capture("online migration job length does not fit memory"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|err| capture(format!("cannot read online migration job: {err}")))?;
    Ok(bytes)
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), MemoryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| capture(format!("cannot create job staging file: {err}")))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|err| capture(format!("cannot sync online migration job: {err}")))
}

fn validate_workspace(workspace: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(workspace)
        .map_err(|err| capture(format!("cannot inspect online migration workspace: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(capture(
            "online migration workspace must be a real directory",
        ));
    }
    Ok(())
}

fn validate_regular_file(path: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| capture(format!("cannot inspect online migration job file: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(capture("online migration job path must be a regular file"));
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, MemoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(capture(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::rename(staging, final_path)
}

#[cfg(windows)]
fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    atomicwrites::replace_atomic(staging, final_path)
}

#[cfg(not(any(unix, windows)))]
fn promote(_staging: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "online migration job replacement unsupported",
    ))
}

#[cfg(unix)]
fn sync_directory(workspace: &Path) -> std::io::Result<()> {
    File::open(workspace)?.sync_all()
}

#[cfg(any(windows, not(any(unix, windows))))]
fn sync_directory(_workspace: &Path) -> std::io::Result<()> {
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
