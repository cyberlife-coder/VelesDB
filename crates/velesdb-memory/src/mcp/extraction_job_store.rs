//! Atomic per-job snapshots for the durable extraction state machine.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::extraction_jobs::{ExtractionJobError, JobRecord};
use crate::mutation::atomic_file::StoreDir;

const JOB_DIRECTORY: &str = "extraction-jobs";
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const TEMP_PREFIX: &str = ".extraction-job-";
/// How many distinct staging names `write_temporary` tries before giving up.
const TEMP_ATTEMPTS: usize = 8;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(super) struct JobStore {
    /// The jobs directory, pinned by an open descriptor at `open` (#2408):
    /// every record open, listing, staging write, and rename resolves a bare
    /// name against it, so a directory swapped in afterwards cannot be
    /// followed.
    dir: StoreDir,
}

impl JobStore {
    pub(super) fn open(root: &Path) -> Result<Self, ExtractionJobError> {
        let directory = root.join(JOB_DIRECTORY);
        ensure_job_directory(root, &directory)?;
        let dir = StoreDir::open(&directory, "extraction jobs").map_err(storage_error)?;
        Ok(Self { dir })
    }

    pub(super) fn pending(&self) -> Result<Vec<String>, ExtractionJobError> {
        let mut pending = Vec::new();
        for name in self.dir.entries().map_err(storage_error)? {
            if self.remove_if_temporary(&name)? {
                continue;
            }
            let record = self.load_entry(&name)?;
            if !record.state.is_terminal() {
                pending.push(record.request_id);
            }
        }
        pending.sort();
        Ok(pending)
    }

    pub(super) fn load(&self, request_id: &str) -> Result<Option<JobRecord>, ExtractionJobError> {
        let name = record_name(request_id);
        let Some(bytes) = read_record_bytes(&self.dir, &name)? else {
            return Ok(None);
        };
        let record: JobRecord = serde_json::from_slice(&bytes).map_err(storage_error)?;
        record.validate()?;
        validate_filename(&record, request_id, &self.dir.path().join(&name))?;
        Ok(Some(record))
    }

    pub(super) fn save(&self, record: &JobRecord) -> Result<(), ExtractionJobError> {
        let bytes = serialize_record(record)?;
        let temporary = write_temporary(&self.dir, &bytes)?;
        self.dir
            .promote(&temporary, &record_name(&record.request_id))
            .map_err(storage_error)?;
        self.dir.sync().map_err(storage_error)
    }

    fn remove_if_temporary(&self, name: &str) -> Result<bool, ExtractionJobError> {
        let is_temporary = name.starts_with(TEMP_PREFIX)
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("tmp"));
        if !is_temporary {
            return Ok(false);
        }
        self.dir.remove(name).map_err(storage_error)?;
        Ok(true)
    }

    fn load_entry(&self, name: &str) -> Result<JobRecord, ExtractionJobError> {
        let request_id = name.strip_suffix(".json").ok_or_else(|| {
            ExtractionJobError::Storage(format!(
                "unexpected entry in {}: {name}",
                self.dir.path().display()
            ))
        })?;
        self.load(request_id)?
            .ok_or_else(|| ExtractionJobError::Storage(format!("job record vanished: {name}")))
    }
}

fn record_name(request_id: &str) -> String {
    format!("{request_id}.json")
}

fn ensure_job_directory(root: &Path, directory: &Path) -> Result<(), ExtractionJobError> {
    match std::fs::create_dir(directory) {
        Ok(()) => sync_directory(root).map_err(storage_error),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(storage_error(error)),
    }
}

fn read_record_bytes(dir: &StoreDir, name: &str) -> Result<Option<Vec<u8>>, ExtractionJobError> {
    // `exists`'s own failure is built by `capture`, which carries
    // `MigrationError::Capture`'s "migration capture error:" framing — meant for
    // the online-migration observer, not this unrelated caller. Unwrapped
    // to its inner detail so a genuine stat failure (e.g. the containing
    // directory losing search permission) keeps its own I/O detail instead
    // (#2409 round 12).
    let exists = dir.exists(name).map_err(|err| match err {
        crate::MemoryError::Migration(crate::migration::MigrationError::Capture(detail)) => {
            storage_error(detail)
        }
        other => storage_error(other),
    })?;
    if !exists {
        return Ok(None);
    }
    let path = dir.path().join(name);
    // A record is never reopened to write into: `save` always creates a
    // fresh temporary file and `rename`s it over this path (`promote`
    // below), so a hard link planted at or before this path's current
    // name has no future write to intercept — the next `save`'s rename
    // replaces the directory entry outright regardless of `nlink`. A
    // hard-link based backup of the whole store (`cp -al`, `rsync
    // --link-dest`) routinely leaves `nlink > 1` on every record it
    // captures, including ones never tampered with, and this store's own
    // startup scan (`pending`) treats any refused record as fatal — so
    // refusing on `nlink` here would break daemon startup after a
    // legitimate backup restore (#2409 round 7).
    // `open_regular_file_allow_hard_links` therefore still refuses a
    // symlink swap (#2404) or any other non-regular type (a FIFO, and on
    // unix a directory too — Windows can't even open a directory here
    // without `FILE_FLAG_BACKUP_SEMANTICS`, which this crate doesn't set,
    // so it surfaces as the generic `MigrationError::Capture` arm below instead,
    // unverified), just not a hard link; such a refusal gets this store's
    // own "invalid extraction job record" phrasing (matching the
    // oversized-record refusal below) instead of the shared guard's
    // internal-sounding wording. An unrelated I/O failure opening or
    // inspecting the path (permission denied, too many open files) is NOT
    // this — it keeps its own detail through `storage_error` rather than
    // being swallowed the same way.
    let (mut file, metadata) = dir
        .open_regular_file_allow_hard_links(name, "extraction job record")
        .map_err(|err| match err {
            crate::MemoryError::NotARegularFile { .. } => ExtractionJobError::Storage(format!(
                "invalid extraction job record {}",
                path.display()
            )),
            crate::MemoryError::Migration(crate::migration::MigrationError::Capture(detail)) => {
                storage_error(detail)
            }
            other => storage_error(other),
        })?;
    let length = metadata.len();
    if length > MAX_RECORD_BYTES {
        return Err(ExtractionJobError::Storage(format!(
            "invalid extraction job record {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(storage_error)?;
    Ok(Some(bytes))
}

fn validate_filename(
    record: &JobRecord,
    request_id: &str,
    path: &Path,
) -> Result<(), ExtractionJobError> {
    if record.request_id != request_id {
        return Err(ExtractionJobError::Storage(format!(
            "job filename does not match its request_id: {}",
            path.display()
        )));
    }
    Ok(())
}

fn serialize_record(record: &JobRecord) -> Result<Vec<u8>, ExtractionJobError> {
    record.validate()?;
    let bytes = serde_json::to_vec(record).map_err(storage_error)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(ExtractionJobError::Storage(format!(
            "job '{}' exceeds the {} byte snapshot limit",
            record.request_id, MAX_RECORD_BYTES
        )));
    }
    Ok(bytes)
}

/// Writes `bytes` to a fresh staging file inside `dir` and returns its
/// *name*. The name is `{TEMP_PREFIX}{pid}-{sequence}.tmp` — `O_EXCL`
/// creation inside the pinned directory retries a collision rather than
/// following or overwriting it, and a `save` never reopens that name
/// afterwards: the handle it was written through is dropped only after
/// `dir.promote` renames it.
fn write_temporary(dir: &StoreDir, bytes: &[u8]) -> Result<String, ExtractionJobError> {
    for _ in 0..TEMP_ATTEMPTS {
        let name = next_temp_name();
        match dir.create_new(&name, 0o600) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(storage_error)?;
                file.flush().map_err(storage_error)?;
                file.sync_all().map_err(storage_error)?;
                return Ok(name);
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(storage_error(err)),
        }
    }
    Err(ExtractionJobError::Storage(
        "cannot allocate an extraction job staging name".to_owned(),
    ))
}

fn next_temp_name() -> String {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{TEMP_PREFIX}{}-{:x}.tmp", std::process::id(), sequence)
}

pub(super) fn storage_error(error: impl std::fmt::Display) -> ExtractionJobError {
    ExtractionJobError::Storage(error.to_string())
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> std::io::Result<()> {
    std::fs::File::open(directory)?.sync_all()
}

#[cfg(windows)]
fn sync_directory(_directory: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_directory: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no durable directory barrier is available on this platform",
    ))
}
