//! Atomic file write: serialize to a unique sibling temp file, fsync, then
//! rename over the target.
//!
//! Shared by every durable snapshot writer (HNSW, BM25, and the graph postcard
//! snapshots — `EdgeStore` / `PropertyIndex` / `RangeIndex`) so the crash-safety
//! guarantee lives in exactly one place instead of being re-implemented per
//! module.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-global counter making temp file names unique within a process.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Writes `data` to `final_path` atomically.
///
/// Serializes to a uniquely-named sibling temp file (same directory → same
/// filesystem, so the `rename` is atomic and cannot fail with `EXDEV`), fsyncs
/// it, then renames it over the target. A crash mid-write leaves the *previous*
/// good file intact rather than a torn file. The temp file is best-effort
/// removed if any step fails.
///
/// The temp name embeds PID + thread id + a process-global counter so
/// concurrent writers (intra- and inter-process) never collide on it.
///
/// # Errors
///
/// Returns an error if creating, writing, or fsyncing the temp file fails, or
/// if the final rename fails.
pub(crate) fn atomic_write(final_path: &Path, data: &[u8]) -> std::io::Result<()> {
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let tid = std::thread::current().id();
    let file_name = final_path.file_name().unwrap_or_default().to_string_lossy();
    let tmp_path = final_path.with_file_name(format!("{file_name}.tmp.{pid}.{tid:?}.{seq}"));

    let result = atomic_write_inner(&tmp_path, final_path, data);
    if result.is_err() {
        // Best-effort cleanup of the temp file on failure.
        let _ = std::fs::remove_file(&tmp_path);
    }
    result
}

fn atomic_write_inner(tmp_path: &Path, final_path: &Path, data: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(tmp_path)?;
    let mut writer = std::io::BufWriter::new(file);
    writer.write_all(data)?;
    writer.flush()?;
    writer.get_ref().sync_all()?; // durable before the rename swaps it in
    std::fs::rename(tmp_path, final_path)
}

#[cfg(test)]
mod tests {
    use super::atomic_write;

    #[test]
    fn test_atomic_write_round_trips_and_leaves_no_temp() {
        let dir = tempfile::TempDir::new().expect("test: temp dir");
        let path = dir.path().join("snap.bin");

        atomic_write(&path, b"hello").expect("test: write");
        assert_eq!(std::fs::read(&path).expect("test: read"), b"hello");

        // No stray temp files remain after a successful write.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .expect("test: read dir")
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "no .tmp files should remain");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let dir = tempfile::TempDir::new().expect("test: temp dir");
        let path = dir.path().join("snap.bin");
        atomic_write(&path, b"first").expect("test: first");
        atomic_write(&path, b"second").expect("test: overwrite");
        assert_eq!(std::fs::read(&path).expect("test: read"), b"second");
    }
}
