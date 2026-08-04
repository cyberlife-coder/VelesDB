use super::filesystem::{bytes_on_disk, fingerprint};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const SCRATCH_PREFIX: &str = ".velesdb-diagnosis-";
const OWNER_MARKER: &str = ".velesdb-diagnostic-owner";
const FIXED_STAGING_HEADROOM: u64 = 16 * 1024 * 1024;
const STAGING_PERCENT_HEADROOM: u64 = 10;
const CREATE_ATTEMPTS: u64 = 64;

static SCRATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A verified point-in-time copy used to keep `Database::open` off the source.
pub(super) struct DiagnosticCopy {
    scratch: ScratchDirectory,
    store_path: PathBuf,
    source_fingerprint: String,
    source_bytes: u64,
    staging_required: u64,
    staging_available: u64,
}

struct CaptureEvidence {
    source_fingerprint: String,
    source_bytes: u64,
    staging_required: u64,
    staging_available: u64,
}

impl DiagnosticCopy {
    /// Capture `source` once, refusing a moving tree or insufficient staging.
    pub(super) fn capture(
        source: &Path,
        scratch_parent: &Path,
    ) -> Result<Self, crate::MemoryError> {
        let probe = |path: &Path| fs2::available_space(path);
        let mut no_hook = |_path: &Path| Ok(());
        Self::capture_with(source, scratch_parent, &probe, &mut no_hook)
    }

    /// The same capture protocol with deterministic seams for failure tests.
    pub(super) fn capture_with(
        source: &Path,
        scratch_parent: &Path,
        available_space: &dyn Fn(&Path) -> std::io::Result<u64>,
        after_file_copy: &mut dyn FnMut(&Path) -> Result<(), crate::MemoryError>,
    ) -> Result<Self, crate::MemoryError> {
        let evidence = prepare_capture(source, scratch_parent, available_space)?;
        let scratch = ScratchDirectory::create(scratch_parent)?;
        let store_path = scratch.path().join("store");
        if let Err(error) = populate_verified_store(
            source,
            &store_path,
            &evidence.source_fingerprint,
            after_file_copy,
        ) {
            return cleanup_after_capture_error(scratch, error);
        }

        Ok(Self {
            scratch,
            store_path,
            source_fingerprint: evidence.source_fingerprint,
            source_bytes: evidence.source_bytes,
            staging_required: evidence.staging_required,
            staging_available: evidence.staging_available,
        })
    }

    pub(super) fn store_path(&self) -> &Path {
        &self.store_path
    }

    pub(super) fn source_fingerprint(&self) -> &str {
        &self.source_fingerprint
    }

    pub(super) fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    pub(super) fn staging_required(&self) -> u64 {
        self.staging_required
    }

    pub(super) fn staging_available(&self) -> u64 {
        self.staging_available
    }

    /// Refuse a report if the live source moved after the copy was captured.
    pub(super) fn verify_source_unchanged(&self, source: &Path) -> Result<(), crate::MemoryError> {
        let current = fingerprint(source)?;
        if current == self.source_fingerprint {
            Ok(())
        } else {
            Err(query_error(format!(
                "source changed during diagnosis: captured fingerprint '{}' but found '{}'; no report is valid, retry once the store is quiescent",
                self.source_fingerprint, current
            )))
        }
    }

    /// Return `result` only after the owned scratch has been removed.
    pub(super) fn finish<T>(
        self,
        result: Result<T, crate::MemoryError>,
    ) -> Result<T, crate::MemoryError> {
        let cleanup = self.scratch.cleanup();
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Err(error), Err(cleanup)) => Err(combined_error(&error, &cleanup)),
        }
    }
}

fn prepare_capture(
    source: &Path,
    scratch_parent: &Path,
    available_space: &dyn Fn(&Path) -> std::io::Result<u64>,
) -> Result<CaptureEvidence, crate::MemoryError> {
    reject_unsafe_scratch_parent(source, scratch_parent)?;
    let source_fingerprint = fingerprint(source)?;
    let source_bytes = bytes_on_disk(source)?;
    let staging_required = staging_requirement(source_bytes)?;
    let staging_available = available_space(scratch_parent).map_err(|err| {
        query_error(format!(
            "cannot establish free space for diagnostic staging at {}: {err}",
            scratch_parent.display()
        ))
    })?;
    if staging_available < staging_required {
        return Err(query_error(format!(
            "insufficient diagnostic staging space at {}: {staging_available} bytes available, {staging_required} required before copying a {source_bytes}-byte store",
            scratch_parent.display()
        )));
    }
    Ok(CaptureEvidence {
        source_fingerprint,
        source_bytes,
        staging_required,
        staging_available,
    })
}

fn populate_verified_store(
    source: &Path,
    store_path: &Path,
    expected_fingerprint: &str,
    after_file_copy: &mut dyn FnMut(&Path) -> Result<(), crate::MemoryError>,
) -> Result<(), crate::MemoryError> {
    create_private_directory(store_path).map_err(|err| {
        query_error(format!(
            "cannot create diagnostic store {}: {err}",
            store_path.display()
        ))
    })?;
    copy_directory(source, store_path, after_file_copy)?;

    let copied_fingerprint = fingerprint(store_path)?;
    let source_after_copy = fingerprint(source)?;
    if copied_fingerprint == expected_fingerprint && source_after_copy == expected_fingerprint {
        Ok(())
    } else {
        Err(source_changed(
            expected_fingerprint,
            &source_after_copy,
            &copied_fingerprint,
        ))
    }
}

fn cleanup_after_capture_error(
    scratch: ScratchDirectory,
    error: crate::MemoryError,
) -> Result<DiagnosticCopy, crate::MemoryError> {
    match scratch.cleanup() {
        Ok(()) => Err(error),
        Err(cleanup) => Err(combined_error(&error, &cleanup)),
    }
}

pub(super) fn staging_requirement(source_bytes: u64) -> Result<u64, crate::MemoryError> {
    let percentage = source_bytes
        .checked_mul(STAGING_PERCENT_HEADROOM)
        .and_then(|value| value.checked_div(100))
        .ok_or_else(|| query_error("diagnostic staging size overflow".to_owned()))?;
    source_bytes
        .checked_add(percentage)
        .and_then(|value| value.checked_add(FIXED_STAGING_HEADROOM))
        .ok_or_else(|| query_error("diagnostic staging size overflow".to_owned()))
}

fn reject_unsafe_scratch_parent(
    source: &Path,
    scratch_parent: &Path,
) -> Result<(), crate::MemoryError> {
    let metadata = std::fs::symlink_metadata(scratch_parent).map_err(|err| {
        query_error(format!(
            "cannot inspect diagnostic staging parent {}: {err}",
            scratch_parent.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(query_error(format!(
            "diagnostic staging parent {} must be a real directory, not a symlink or special file",
            scratch_parent.display()
        )));
    }
    let source = std::fs::canonicalize(source)
        .map_err(|err| query_error(format!("cannot resolve source {}: {err}", source.display())))?;
    let scratch_parent = std::fs::canonicalize(scratch_parent).map_err(|err| {
        query_error(format!(
            "cannot resolve diagnostic staging parent {}: {err}",
            scratch_parent.display()
        ))
    })?;
    if scratch_parent.starts_with(&source) {
        return Err(query_error(format!(
            "diagnostic staging parent {} is inside source {}; copying there would mutate the tree being verified",
            scratch_parent.display(),
            source.display()
        )));
    }
    Ok(())
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    after_file_copy: &mut dyn FnMut(&Path) -> Result<(), crate::MemoryError>,
) -> Result<(), crate::MemoryError> {
    let mut entries = std::fs::read_dir(source)
        .map_err(|err| query_error(format!("cannot read {}: {err}", source.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| {
            query_error(format!(
                "cannot read an entry in {}: {err}",
                source.display()
            ))
        })?;
    entries.sort_unstable_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&from)
            .map_err(|err| query_error(format!("cannot inspect {}: {err}", from.display())))?;
        if metadata.file_type().is_symlink() {
            return Err(query_error(format!(
                "migration source contains symlink {}; refusing to follow data outside the tree",
                from.display()
            )));
        }
        if metadata.is_dir() {
            create_private_directory(&to).map_err(|err| {
                query_error(format!(
                    "cannot create diagnostic directory {}: {err}",
                    to.display()
                ))
            })?;
            copy_directory(&from, &to, after_file_copy)?;
        } else if metadata.is_file() {
            copy_regular_file(&from, &to, metadata.len())?;
            after_file_copy(&from)?;
        } else {
            return Err(query_error(format!(
                "migration source contains special file {}; only directories and regular files are supported",
                from.display()
            )));
        }
    }
    Ok(())
}

fn copy_regular_file(
    source: &Path,
    destination: &Path,
    expected_len: u64,
) -> Result<(), crate::MemoryError> {
    let mut input = File::open(source)
        .map_err(|err| query_error(format!("cannot open {}: {err}", source.display())))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|err| query_error(format!("cannot create {}: {err}", destination.display())))?;
    let copied = std::io::copy(&mut input, &mut output).map_err(|err| {
        query_error(format!(
            "cannot copy {} to {}: {err}",
            source.display(),
            destination.display()
        ))
    })?;
    output.flush().map_err(|err| {
        query_error(format!(
            "cannot flush diagnostic copy {}: {err}",
            destination.display()
        ))
    })?;
    if copied != expected_len {
        return Err(query_error(format!(
            "source changed while copying {}: expected {expected_len} bytes, copied {copied}",
            source.display()
        )));
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}

struct ScratchDirectory {
    path: PathBuf,
    owner_token: String,
    cleanup_attempted: bool,
}

impl ScratchDirectory {
    fn create(parent: &Path) -> Result<Self, crate::MemoryError> {
        for _ in 0..CREATE_ATTEMPTS {
            let token = scratch_token();
            let path = parent.join(format!("{SCRATCH_PREFIX}{token}"));
            match create_private_directory(&path) {
                Ok(()) => {
                    let marker = path.join(OWNER_MARKER);
                    let marker_result = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&marker)
                        .and_then(|mut file| {
                            file.write_all(token.as_bytes())?;
                            file.flush()
                        });
                    if let Err(err) = marker_result {
                        let _ = std::fs::remove_dir_all(&path);
                        return Err(query_error(format!(
                            "cannot mark owned diagnostic scratch {}: {err}",
                            path.display()
                        )));
                    }
                    return Ok(Self {
                        path,
                        owner_token: token,
                        cleanup_attempted: false,
                    });
                }
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(err) => {
                    return Err(query_error(format!(
                        "cannot create diagnostic scratch under {}: {err}",
                        parent.display()
                    )))
                }
            }
        }
        Err(query_error(format!(
            "cannot allocate a unique diagnostic scratch under {} after {CREATE_ATTEMPTS} attempts",
            parent.display()
        )))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(mut self) -> Result<(), crate::MemoryError> {
        self.cleanup_attempted = true;
        if !self.is_owned() {
            return Err(query_error(format!(
                "refusing to remove diagnostic scratch {} because its ownership marker is missing or changed; inspect and remove that exact directory manually",
                self.path.display()
            )));
        }
        std::fs::remove_dir_all(&self.path).map_err(|err| {
            query_error(format!(
                "cannot fully remove diagnostic scratch {}: {err}; inspect that exact path because cleanup may be partial",
                self.path.display()
            ))
        })
    }

    fn is_owned(&self) -> bool {
        let mut marker = String::new();
        File::open(self.path.join(OWNER_MARKER))
            .and_then(|mut file| file.read_to_string(&mut marker))
            .is_ok()
            && marker == self.owner_token
    }
}

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        if !self.cleanup_attempted && self.is_owned() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn scratch_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let sequence = SCRATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{sequence}", std::process::id())
}

fn source_changed(expected: &str, source_after: &str, copied: &str) -> crate::MemoryError {
    query_error(format!(
        "source changed while the diagnostic copy was captured: expected '{expected}', source ended as '{source_after}', copy is '{copied}'; no report is valid, retry once the store is quiescent"
    ))
}

fn combined_error(
    primary: &crate::MemoryError,
    cleanup: &crate::MemoryError,
) -> crate::MemoryError {
    query_error(format!(
        "{primary}; additionally, scratch cleanup failed: {cleanup}"
    ))
}

fn query_error(message: String) -> crate::MemoryError {
    velesdb_core::Error::Query(message).into()
}
