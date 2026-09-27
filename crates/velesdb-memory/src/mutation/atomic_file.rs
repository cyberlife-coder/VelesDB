//! Shared atomic-replace primitives for small on-disk state/record files:
//! reject a symlinked target, open one race-free, promote a staging file
//! into place, and durability-barrier the containing directory.
//! `mutation::controller` and `online_migration` each keep one such file and
//! used to carry their own copy of these functions.

use std::fs::{File, OpenOptions};
use std::path::Path;

use crate::MemoryError;

pub(crate) fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}

pub(crate) fn validate_workspace(path: &Path, entity: &str) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| capture(format!("cannot inspect {entity} workspace: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(capture(format!(
            "{entity} workspace must be a real directory"
        )));
    }
    Ok(())
}

pub(crate) fn validate_regular_file(path: &Path, entity: &str) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| capture(format!("cannot inspect {entity} file: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(capture(format!("{entity} path must be a regular file")));
    }
    Ok(())
}

/// Opens `path` for `entity`, refusing it if it names a symlink or anything
/// but a regular file. The refusal is decided from the file this call opens,
/// not from a separate `stat` of the path beforehand: `validate_regular_file`
/// followed by a plain `open` leaves a window where a link swapped in
/// between the two is followed by the open (#2404).
pub(crate) fn open_regular_file(
    path: &Path,
    entity: &str,
    mut options: OpenOptions,
) -> Result<File, MemoryError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // `O_NONBLOCK` alongside `O_NOFOLLOW`: opening a FIFO would otherwise
        // block the caller until a peer opens the other end (#2404 follow-up
        // finding), turning a planted FIFO into a hang instead of the
        // rejection below. It has no effect on a regular file (POSIX
        // open(2)), so a legitimate open is unaffected.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Opens a reparse point itself rather than its target, so a symlink
        // swapped in surfaces as a non-regular file in the check below
        // instead of being read through.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options
        .open(path)
        .map_err(|err| open_error(&err, path, entity))?;
    let is_file = file
        .metadata()
        .map_err(|err| capture(format!("cannot inspect {entity} file: {err}")))?
        .is_file();
    if !is_file {
        return Err(capture(format!("{entity} path must be a regular file")));
    }
    Ok(file)
}

#[cfg(unix)]
fn open_error(err: &std::io::Error, path: &Path, entity: &str) -> MemoryError {
    // ELOOP: `O_NOFOLLOW` refused a symlink. EISDIR: opening a directory for
    // write fails here rather than at a separate pre-check — dropping
    // `prepare_journal`'s redundant `validate_regular_file` (this PR) means a
    // directory at the journal path now surfaces through this open instead.
    // Both name the same refusal a caller already handles.
    if matches!(err.raw_os_error(), Some(libc::ELOOP | libc::EISDIR)) {
        capture(format!("{entity} path must be a regular file"))
    } else {
        capture(format!(
            "cannot open {entity} file {}: {err}",
            path.display()
        ))
    }
}

#[cfg(not(unix))]
fn open_error(err: &std::io::Error, path: &Path, entity: &str) -> MemoryError {
    capture(format!(
        "cannot open {entity} file {}: {err}",
        path.display()
    ))
}

pub(crate) fn path_exists(path: &Path) -> Result<bool, MemoryError> {
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
pub(crate) fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::rename(staging, final_path)
}

#[cfg(windows)]
pub(crate) fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    atomicwrites::replace_atomic(staging, final_path)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn promote(_staging: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic file replacement unsupported",
    ))
}

#[cfg(unix)]
pub(crate) fn sync_directory(workspace: &Path) -> std::io::Result<()> {
    std::fs::File::open(workspace)?.sync_all()
}

#[cfg(any(windows, not(any(unix, windows))))]
pub(crate) fn sync_directory(_workspace: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "atomic_file_tests.rs"]
mod atomic_file_tests;
