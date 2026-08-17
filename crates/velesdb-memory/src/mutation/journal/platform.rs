use std::path::Path;

#[cfg(unix)]
pub(super) fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::rename(staging, final_path)
}

#[cfg(windows)]
pub(super) fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    atomicwrites::replace_atomic(staging, final_path)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn promote(_staging: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "journal replacement unsupported",
    ))
}

#[cfg(unix)]
pub(super) fn durability_barrier(workspace: &Path) -> std::io::Result<()> {
    std::fs::File::open(workspace)?.sync_all()
}

#[cfg(any(windows, not(any(unix, windows))))]
pub(super) fn durability_barrier(_workspace: &Path) -> std::io::Result<()> {
    Ok(())
}
