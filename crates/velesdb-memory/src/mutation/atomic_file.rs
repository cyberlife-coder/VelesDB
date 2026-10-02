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

/// A deliberate refusal — the path opened or inspected is not usable as a
/// lone regular file (a symlink, a hard link, a directory, a socket, ...) —
/// kept distinct from [`capture`]'s generic internal-failure wording so a
/// caller can match on it and apply its own phrasing without also
/// swallowing an unrelated I/O failure (permission denied, too many open
/// files) the same call can raise.
fn not_a_regular_file(entity: &str) -> MemoryError {
    MemoryError::NotARegularFile {
        entity: entity.to_owned(),
    }
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

/// Opens `path` for `entity`, refusing it if it names a symlink, a hard
/// link, or anything but a lone regular file. The refusal is decided from
/// the file this call opens, not from a separate `stat` of the path
/// beforehand: `validate_regular_file` followed by a plain `open` leaves a
/// window where a link swapped in between the two is followed by the open
/// (#2404).
///
/// Refusing the hard-link case only makes sense for a file this store
/// writes into through a handle it keeps reopening across its lifetime (a
/// lock, a journal, a controller-state file): a legitimate copy of it is
/// never hard linked, so `nlink > 1` can only mean a link planted before or
/// at this open, ready to intercept a future write through that same name
/// (#2407). A file this store only ever replaces wholesale — by renaming a
/// fresh temporary file over it, never reopening the old name to write into
/// it — has no such window: the rename severs any hard link planted before
/// it, so the content a later read sees is always the legitimate one
/// regardless of `nlink` (an extraction job record is exactly this shape —
/// #2409 round 7). That case wants [`open_regular_file_allow_hard_links`]
/// instead, same as a path this helper only ever *reads* from someone
/// else's tree, where a hard-link based backup (`cp -al`,
/// `rsync --link-dest`) routinely produces `nlink > 1` on perfectly
/// ordinary files.
pub(crate) fn open_regular_file(
    path: &Path,
    entity: &str,
    options: OpenOptions,
) -> Result<File, MemoryError> {
    let (file, metadata) = open_checked(path, entity, options)?;
    #[cfg(not(unix))]
    {
        let _ = metadata;
    }
    #[cfg(unix)]
    {
        // A hard link to a file outside the store is, structurally, an
        // ordinary regular file: the `is_file` check in `open_checked`
        // cannot tell it apart, and `O_NOFOLLOW` only ever guarded the
        // symlink case (#2407). `nlink == 0` is not that: a reader can win
        // the open() race against a concurrent, legitimate rename-replace of
        // the same path by another writer, which unlinks the name this
        // handle held without touching the content already read through it
        // — refusing that would turn a routine concurrent read into a
        // spurious failure.
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() > 1 {
            return Err(not_a_regular_file(entity));
        }
    }
    Ok(file)
}

/// As [`open_regular_file`], but accepts a hard-linked target. Fits two
/// shapes: a path this helper only reads from a tree it does not own (a
/// diagnostic source directory), and a path this store owns but only ever
/// replaces wholesale by rename rather than reopening to write into (an
/// extraction job record). Either way a hard link planted by a backup tool
/// is indistinguishable from an ordinary file, so there is nothing here to
/// refuse it against. The symlink-swap protection (#2404) still applies in
/// full.
pub(crate) fn open_regular_file_allow_hard_links(
    path: &Path,
    entity: &str,
    options: OpenOptions,
) -> Result<File, MemoryError> {
    let (file, _metadata) = open_checked(path, entity, options)?;
    Ok(file)
}

/// Opens `path` with the symlink-swap protection (#2404) and returns the
/// already-fetched `Metadata` alongside the handle, so a caller that also
/// needs to inspect it (the hard-link check above) does not `fstat` the
/// same handle a second time.
fn open_checked(
    path: &Path,
    entity: &str,
    mut options: OpenOptions,
) -> Result<(File, std::fs::Metadata), MemoryError> {
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
    let metadata = file.metadata().map_err(|err| {
        capture(format!(
            "cannot inspect {entity} file {}: {err}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(not_a_regular_file(entity));
    }
    Ok((file, metadata))
}

#[cfg(unix)]
fn open_error(err: &std::io::Error, path: &Path, entity: &str) -> MemoryError {
    // ELOOP: `O_NOFOLLOW` refused a symlink, a fast path with no further
    // lookup needed. Anything else that isn't a regular file (a directory
    // via `EISDIR`, a socket, and so on) fails `open` itself with a
    // platform- and file-type-specific errno that isn't worth enumerating
    // one by one, so this falls back to a `symlink_metadata` lookup that
    // only chooses which message reports the already-decided refusal —
    // `open` already returned `Err`, so this decides nothing
    // security-relevant, unlike the checks in `open_regular_file` that run
    // on an already-open handle.
    if err.raw_os_error() == Some(libc::ELOOP) {
        return not_a_regular_file(entity);
    }
    let is_non_regular = std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file());
    if is_non_regular {
        not_a_regular_file(entity)
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
