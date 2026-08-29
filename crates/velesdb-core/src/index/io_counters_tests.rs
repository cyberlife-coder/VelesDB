use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard};

static OPENS: AtomicU32 = AtomicU32::new(0);
static FLUSHES: AtomicU32 = AtomicU32::new(0);
static SYNCS: AtomicU32 = AtomicU32::new(0);
static WATCH: Mutex<Option<PathBuf>> = Mutex::new(None);
static SERIALISE: Mutex<()> = Mutex::new(());

/// Successful WAL syscalls observed on one WAL file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WalIoCounts {
    /// Appends that opened the file.
    pub opens: u32,
    /// Buffer flushes that succeeded.
    pub flushes: u32,
    /// `sync_all` calls that succeeded — the durability barriers.
    pub syncs: u32,
}

/// Whether this exact WAL file is the one under observation.
///
/// Filtering on the PATH, not merely on the `context`, is what makes these
/// counts safe under `cargo test`'s default parallelism: every test writes
/// into its own temporary directory, so a neighbouring test indexing its own
/// documents cannot inflate the numbers. Filtering on the context alone left
/// the counters shared by every BM25 WAL in the process, which made the
/// assertions pass single-threaded and fail in parallel.
fn watched(wal_path: &Path) -> bool {
    WATCH
        .lock()
        .map(|w| w.as_deref() == Some(wal_path))
        .unwrap_or(false)
}

pub(super) fn record_open(wal_path: &Path, _context: &str) {
    if watched(wal_path) {
        OPENS.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn record_flush(wal_path: &Path, _context: &str) {
    if watched(wal_path) {
        FLUSHES.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn record_sync(wal_path: &Path, _context: &str) {
    if watched(wal_path) {
        SYNCS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Disarms the watch even if the observed closure panics.
struct Watch<'a>(#[allow(dead_code)] MutexGuard<'a, ()>);

impl Drop for Watch<'_> {
    fn drop(&mut self) {
        if let Ok(mut w) = WATCH.lock() {
            *w = None;
        }
    }
}

/// Runs `f`, counting the WAL syscalls it performs on `wal_path`.
///
/// Counts are taken AFTER each syscall succeeds, so a failed open or fsync
/// is not counted: what is measured is durable work, not attempts.
#[expect(clippy::significant_drop_tightening)] // Reason: the guard under test is held to the assertion on purpose
pub(crate) fn count_wal_io<T>(wal_path: &Path, f: impl FnOnce() -> T) -> (T, WalIoCounts) {
    let guard = SERIALISE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Ok(mut w) = WATCH.lock() {
        *w = Some(wal_path.to_path_buf());
    }
    OPENS.store(0, Ordering::Relaxed);
    FLUSHES.store(0, Ordering::Relaxed);
    SYNCS.store(0, Ordering::Relaxed);
    let _watch = Watch(guard);
    let out = f();
    let counts = WalIoCounts {
        opens: OPENS.load(Ordering::Relaxed),
        flushes: FLUSHES.load(Ordering::Relaxed),
        syncs: SYNCS.load(Ordering::Relaxed),
    };
    (out, counts)
}
