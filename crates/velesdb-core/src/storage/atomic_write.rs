//! Power-loss-durable atomic file replacement.
//!
//! Shared by every durable snapshot writer (HNSW, BM25, and the graph postcard
//! snapshots — `EdgeStore` / `PropertyIndex` / `RangeIndex`) so the crash-safety
//! guarantee lives in exactly one place instead of being re-implemented per
//! module.

use std::io::{self, Write};
use std::path::Path;

#[cfg(test)]
use std::cell::Cell;

/// Writes `data` to `final_path` atomically.
///
/// Serializes to a securely-created, uniquely-named sibling temp file (same
/// directory → same filesystem, so the replacement is atomic and cannot fail
/// with `EXDEV`), fsyncs it, replaces the target, then persists that namespace
/// change. Unix uses a parent-directory fsync; Windows uses
/// `MOVEFILE_WRITE_THROUGH`.
/// A crash mid-write therefore leaves either the previous complete file or the
/// replacement, never a torn file. The temp file is best-effort removed if any
/// step fails.
///
/// # Errors
///
/// Returns an error if any write, file sync, replacement, or namespace
/// durability barrier fails.
pub(crate) fn atomic_write(final_path: &Path, data: &[u8]) -> io::Result<()> {
    atomic_write_with(final_path, |writer| writer.write_all(data))
}

/// Streams a replacement file through `write`, then publishes it durably.
///
/// This variant avoids materializing large snapshots in memory. The closure's
/// result is returned only after the file and its replacement are durable.
///
/// # Errors
///
/// Returns an error from the writer or any durability boundary.
pub(crate) fn atomic_write_with<T, E>(
    final_path: &Path,
    write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<io::Error>,
{
    let (file, tmp_path) = create_temporary_file(final_path)?;
    atomic_write_inner(file, tmp_path.as_ref(), final_path, write)
}

fn create_temporary_file(final_path: &Path) -> io::Result<(std::fs::File, tempfile::TempPath)> {
    let file_name = final_path.file_name().unwrap_or_default().to_string_lossy();
    let prefix = format!("{file_name}.tmp.");
    tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent_directory(final_path))
        .map(tempfile::NamedTempFile::into_parts)
}

fn atomic_write_inner<T, E>(
    file: std::fs::File,
    tmp_path: &Path,
    final_path: &Path,
    write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> Result<T, E>,
) -> Result<T, E>
where
    E: From<io::Error>,
{
    let mut writer = std::io::BufWriter::new(file);
    let value = write(&mut writer)?;
    writer.flush()?;
    sync_temporary_file(writer.get_ref())?;
    drop(writer);
    replace_file(tmp_path, final_path)?;
    sync_parent_directory(final_path)?;
    Ok(value)
}

fn sync_temporary_file(file: &std::fs::File) -> io::Result<()> {
    check_fault(AtomicWriteBoundary::TemporaryFileSync)?;
    file.sync_all()
}

#[cfg(windows)]
fn replace_file(tmp_path: &Path, final_path: &Path) -> io::Result<()> {
    check_fault(AtomicWriteBoundary::Replacement)?;
    atomicwrites::replace_atomic(tmp_path, final_path)
}

#[cfg(not(windows))]
fn replace_file(tmp_path: &Path, final_path: &Path) -> io::Result<()> {
    check_fault(AtomicWriteBoundary::Replacement)?;
    std::fs::rename(tmp_path, final_path)
}

#[cfg(unix)]
pub(crate) fn sync_parent_directory(final_path: &Path) -> io::Result<()> {
    check_fault(AtomicWriteBoundary::ParentDirectorySync)?;
    std::fs::File::open(parent_directory(final_path))?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_parent_directory(_final_path: &Path) -> io::Result<()> {
    Ok(())
}

fn parent_directory(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AtomicWriteBoundary {
    TemporaryFileSync,
    Replacement,
    #[cfg(unix)]
    ParentDirectorySync,
}

#[cfg(not(test))]
#[allow(clippy::unnecessary_wraps)] // Test builds return injected durability failures.
fn check_fault(_boundary: AtomicWriteBoundary) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
thread_local! {
    static INJECTED_FAULT: Cell<Option<InjectedFault>> = const { Cell::new(None) };
}

#[cfg(test)]
fn check_fault(boundary: AtomicWriteBoundary) -> io::Result<()> {
    INJECTED_FAULT.with(|fault| {
        let Some(mut injected) = fault.get() else {
            return Ok(());
        };
        if injected.boundary != boundary {
            return Ok(());
        }
        if injected.remaining == 0 {
            fault.set(None);
            Err(io::Error::other(format!("fault injected at {boundary:?}")))
        } else {
            injected.remaining -= 1;
            fault.set(Some(injected));
            Ok(())
        }
    })
}

#[cfg(test)]
#[derive(Clone, Copy)]
struct InjectedFault {
    boundary: AtomicWriteBoundary,
    remaining: usize,
}

#[cfg(test)]
pub(crate) struct FaultGuard(Option<InjectedFault>);

#[cfg(test)]
impl FaultGuard {
    pub(crate) fn inject(boundary: AtomicWriteBoundary) -> Self {
        Self::inject_after(boundary, 0)
    }

    pub(crate) fn inject_after(boundary: AtomicWriteBoundary, remaining: usize) -> Self {
        let injected = InjectedFault {
            boundary,
            remaining,
        };
        let previous = INJECTED_FAULT.with(|fault| fault.replace(Some(injected)));
        Self(previous)
    }
}

#[cfg(test)]
impl Drop for FaultGuard {
    fn drop(&mut self) {
        INJECTED_FAULT.with(|fault| fault.set(self.0));
    }
}

#[cfg(test)]
#[path = "atomic_write_tests.rs"]
mod tests;
