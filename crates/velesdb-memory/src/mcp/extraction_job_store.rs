//! Atomic per-job snapshots for the durable extraction state machine.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::extraction_jobs::{JobError, JobRecord};

const JOB_DIRECTORY: &str = "extraction-jobs";
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
const TEMP_PREFIX: &str = ".extraction-job-";

pub(super) struct JobStore {
    directory: PathBuf,
}

impl JobStore {
    pub(super) fn open(root: &Path) -> Result<Self, JobError> {
        let directory = root.join(JOB_DIRECTORY);
        ensure_job_directory(root, &directory)?;
        let metadata = std::fs::symlink_metadata(&directory).map_err(storage_error)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(JobError::Storage(format!(
                "{} must be a real directory, not a symlink",
                directory.display()
            )));
        }
        Ok(Self { directory })
    }

    pub(super) fn pending(&self) -> Result<Vec<String>, JobError> {
        let mut pending = Vec::new();
        for entry in self.entries()? {
            if remove_if_temporary(&entry)? {
                continue;
            }
            let record = self.load_entry(&entry)?;
            if !record.state.is_terminal() {
                pending.push(record.request_id);
            }
        }
        pending.sort();
        Ok(pending)
    }

    pub(super) fn load(&self, request_id: &str) -> Result<Option<JobRecord>, JobError> {
        let path = self.path(request_id);
        let Some(bytes) = read_record_bytes(&path)? else {
            return Ok(None);
        };
        let record: JobRecord = serde_json::from_slice(&bytes).map_err(storage_error)?;
        record.validate()?;
        validate_filename(&record, request_id, &path)?;
        Ok(Some(record))
    }

    pub(super) fn save(&self, record: &JobRecord) -> Result<(), JobError> {
        let bytes = serialize_record(record)?;
        let temporary_path = write_temporary(&self.directory, &bytes)?;
        promote(&temporary_path, &self.path(&record.request_id)).map_err(storage_error)?;
        sync_directory(&self.directory).map_err(storage_error)
    }

    fn entries(&self) -> Result<Vec<std::fs::DirEntry>, JobError> {
        let entries = std::fs::read_dir(&self.directory).map_err(storage_error)?;
        entries.map(|entry| entry.map_err(storage_error)).collect()
    }

    fn load_entry(&self, entry: &std::fs::DirEntry) -> Result<JobRecord, JobError> {
        let name = entry.file_name().to_string_lossy().into_owned();
        let request_id = name.strip_suffix(".json").ok_or_else(|| {
            JobError::Storage(format!(
                "unexpected entry in {}: {name}",
                self.directory.display()
            ))
        })?;
        self.load(request_id)?
            .ok_or_else(|| JobError::Storage(format!("job record vanished: {name}")))
    }

    fn path(&self, request_id: &str) -> PathBuf {
        self.directory.join(format!("{request_id}.json"))
    }
}

fn ensure_job_directory(root: &Path, directory: &Path) -> Result<(), JobError> {
    match std::fs::create_dir(directory) {
        Ok(()) => sync_directory(root).map_err(storage_error),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(storage_error(error)),
    }
}

fn remove_if_temporary(entry: &std::fs::DirEntry) -> Result<bool, JobError> {
    let name = entry.file_name().to_string_lossy().into_owned();
    let is_temporary = name.starts_with(TEMP_PREFIX)
        && entry
            .path()
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tmp"));
    if !is_temporary {
        return Ok(false);
    }
    std::fs::remove_file(entry.path()).map_err(storage_error)?;
    Ok(true)
}

fn read_record_bytes(path: &Path) -> Result<Option<Vec<u8>>, JobError> {
    // `path_exists`'s own failure is built by `capture`, which carries
    // `MigrationCapture`'s "migration capture error:" framing — meant for
    // the online-migration observer, not this unrelated caller. Unwrapped
    // to its inner detail so a genuine stat failure (e.g. the containing
    // directory losing search permission) keeps its own I/O detail instead
    // (#2409 round 12).
    let exists = crate::mutation::atomic_file::path_exists(path).map_err(|err| match err {
        crate::MemoryError::MigrationCapture(detail) => storage_error(detail),
        other => storage_error(other),
    })?;
    if !exists {
        return Ok(None);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
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
    // so it surfaces as the generic `MigrationCapture` arm below instead,
    // unverified), just not a hard link; such a refusal gets this store's
    // own "invalid extraction job record" phrasing (matching the
    // oversized-record refusal below) instead of the shared guard's
    // internal-sounding wording. An unrelated I/O failure opening or
    // inspecting the path (permission denied, too many open files) is NOT
    // this — it keeps its own detail through `storage_error` rather than
    // being swallowed the same way.
    let (mut file, metadata) = crate::mutation::atomic_file::open_regular_file_allow_hard_links(
        path,
        "extraction job record",
        options,
    )
    .map_err(|err| match err {
        crate::MemoryError::NotARegularFile { .. } => {
            JobError::Storage(format!("invalid extraction job record {}", path.display()))
        }
        crate::MemoryError::MigrationCapture(detail) => storage_error(detail),
        other => storage_error(other),
    })?;
    let length = metadata.len();
    if length > MAX_RECORD_BYTES {
        return Err(JobError::Storage(format!(
            "invalid extraction job record {}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(storage_error)?;
    Ok(Some(bytes))
}

fn validate_filename(record: &JobRecord, request_id: &str, path: &Path) -> Result<(), JobError> {
    if record.request_id != request_id {
        return Err(JobError::Storage(format!(
            "job filename does not match its request_id: {}",
            path.display()
        )));
    }
    Ok(())
}

fn serialize_record(record: &JobRecord) -> Result<Vec<u8>, JobError> {
    record.validate()?;
    let bytes = serde_json::to_vec(record).map_err(storage_error)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(JobError::Storage(format!(
            "job '{}' exceeds the {} byte snapshot limit",
            record.request_id, MAX_RECORD_BYTES
        )));
    }
    Ok(bytes)
}

fn write_temporary(directory: &Path, bytes: &[u8]) -> Result<PathBuf, JobError> {
    let mut temporary = tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .suffix(".tmp")
        .tempfile_in(directory)
        .map_err(storage_error)?;
    temporary.write_all(bytes).map_err(storage_error)?;
    temporary.flush().map_err(storage_error)?;
    temporary.as_file().sync_all().map_err(storage_error)?;
    let (file, temporary_path) = temporary
        .keep()
        .map_err(|error| storage_error(error.error))?;
    drop(file);
    Ok(temporary_path)
}

pub(super) fn storage_error(error: impl std::fmt::Display) -> JobError {
    JobError::Storage(error.to_string())
}

#[cfg(unix)]
fn promote(temporary: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::rename(temporary, final_path)
}

#[cfg(windows)]
fn promote(temporary: &Path, final_path: &Path) -> std::io::Result<()> {
    atomicwrites::replace_atomic(temporary, final_path)
}

#[cfg(not(any(unix, windows)))]
fn promote(_temporary: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "durable extraction jobs require Unix or Windows atomic replacement",
    ))
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
