//! Shared atomic-replace primitives for small on-disk state/record files:
//! reject a symlinked target, promote a staging file into place, and
//! durability-barrier the containing directory. `mutation::controller` and
//! `online_migration` each keep one such file and used to carry their own
//! copy of these five functions.

use std::fs::File;
use std::path::Path;

use crate::MemoryError;

fn capture(message: impl Into<String>) -> MemoryError {
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
    File::open(workspace)?.sync_all()
}

#[cfg(any(windows, not(any(unix, windows))))]
pub(crate) fn sync_directory(_workspace: &Path) -> std::io::Result<()> {
    Ok(())
}
