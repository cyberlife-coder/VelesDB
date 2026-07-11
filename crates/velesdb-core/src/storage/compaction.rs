//! Storage compaction for reclaiming space from deleted vectors.
//!
//! This module provides compaction functionality for `MmapStorage`,
//! allowing reclamation of disk space from deleted vectors.
//!
//! # EPIC-033/US-003: Disk Hole-Punch
//!
//! Two strategies are available:
//! - **Full compaction**: Rewrites entire file (best for high fragmentation)
//! - **Hole-punch**: Releases disk blocks in-place (best for sparse deletions)
//!
//! Hole-punch uses:
//! - Linux: `fallocate(FALLOC_FL_PUNCH_HOLE)`
//! - Windows: `FSCTL_SET_ZERO_DATA`

// Reason: Numeric casts in this file are intentional and bounded.
// Each cast site carries an inline #[allow] with a per-site justification.

use super::sharded_index::ShardedIndex;
use memmap2::MmapMut;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

// =========================================================================
// EPIC-033/US-003: Hole-Punch Implementation
// =========================================================================

/// Punches a hole in a file, releasing disk blocks for the specified range.
///
/// This operation zeros the data and releases the underlying disk blocks
/// back to the filesystem, reducing actual disk usage without changing file size.
///
/// # Platform Support
///
/// - **Linux**: Uses `fallocate(FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE)`
/// - **Windows**: Uses `FSCTL_SET_ZERO_DATA` DeviceIoControl
/// - **Other**: Falls back to writing zeros (no disk reclamation)
///
/// # Arguments
///
/// * `file` - Open file handle (must have write access)
/// * `offset` - Start offset of the hole
/// * `len` - Length of the hole in bytes
///
/// # Returns
///
/// `true` if disk space was actually reclaimed, `false` if only zeroed.
#[allow(unused_variables)]
pub fn punch_hole(file: &File, offset: u64, len: u64) -> io::Result<bool> {
    // Zero-length punch is a no-op on every platform. Return early to avoid
    // EINVAL from fallocate(2) on Linux and undefined behaviour from
    // FSCTL_SET_ZERO_DATA when file_offset == beyond_final_zero on Windows.
    if len == 0 {
        return Ok(false);
    }

    #[cfg(target_os = "linux")]
    {
        punch_hole_linux(file, offset, len)
    }

    #[cfg(target_os = "windows")]
    {
        punch_hole_windows(file, offset, len)
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        // Fallback: just zero the region (no disk reclamation)
        punch_hole_fallback(file, offset, len)
    }
}

/// Linux implementation using fallocate with FALLOC_FL_PUNCH_HOLE.
#[cfg(target_os = "linux")]
fn punch_hole_linux(file: &File, offset: u64, len: u64) -> io::Result<bool> {
    use std::os::unix::io::AsRawFd;

    // FALLOC_FL_PUNCH_HOLE = 0x02, FALLOC_FL_KEEP_SIZE = 0x01
    const FALLOC_FL_KEEP_SIZE: i32 = 0x01;
    const FALLOC_FL_PUNCH_HOLE: i32 = 0x02;

    let fd = file.as_raw_fd();
    let mode = FALLOC_FL_PUNCH_HOLE | FALLOC_FL_KEEP_SIZE;
    let offset_off_t = libc::off_t::try_from(offset).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "offset does not fit in libc::off_t",
        )
    })?;
    let len_off_t = libc::off_t::try_from(len).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "len does not fit in libc::off_t",
        )
    })?;

    // SAFETY: `libc::fallocate` requires a valid fd and offsets.
    // - Condition 1: `fd` comes from `file.as_raw_fd()` on an open file handle.
    // - Condition 2: `offset`/`len` are caller-provided ranges for the same file.
    // SAFETY: Hole punching is only exposed through this syscall on Linux.
    let ret = unsafe { libc::fallocate(fd, mode, offset_off_t, len_off_t) };

    if ret == 0 {
        Ok(true) // Disk space reclaimed
    } else {
        let err = io::Error::last_os_error();
        // EOPNOTSUPP means filesystem doesn't support hole punching
        if err.raw_os_error() == Some(libc::EOPNOTSUPP) {
            // Fall back to zeroing
            punch_hole_fallback(file, offset, len)
        } else {
            Err(err)
        }
    }
}

/// Windows implementation using FSCTL_SET_ZERO_DATA.
#[cfg(target_os = "windows")]
fn punch_hole_windows(file: &File, offset: u64, len: u64) -> io::Result<bool> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{FALSE, HANDLE};
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_ZERO_DATA;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    #[repr(C)]
    struct FileZeroDataInformation {
        file_offset: i64,
        beyond_final_zero: i64,
    }

    let handle = file.as_raw_handle() as HANDLE;
    // Reason: Win32 API requires i64 for file offsets. offset and len are typically < i64::MAX
    // on any realistic file system. Saturate to prevent undefined behavior on edge cases.
    #[allow(clippy::cast_possible_wrap)]
    let info = FileZeroDataInformation {
        file_offset: i64::try_from(offset).unwrap_or(i64::MAX),
        beyond_final_zero: i64::try_from(offset.saturating_add(len)).unwrap_or(i64::MAX),
    };

    let mut bytes_returned: u32 = 0;

    // SAFETY: `DeviceIoControl` requires valid handle/argument pointers.
    // - Condition 1: `handle` comes from `file.as_raw_handle()` for an open file.
    // - Condition 2: `info` and `bytes_returned` pointers are valid for the call duration.
    // SAFETY: Windows sparse-zero operation is only reachable via this API.
    let result = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_ZERO_DATA,
            std::ptr::addr_of!(info).cast(),
            // Reason: FileZeroDataInformation struct size is always <= 16 bytes; fits in u32.
            #[allow(clippy::cast_possible_truncation)]
            {
                std::mem::size_of::<FileZeroDataInformation>() as u32
            },
            std::ptr::null_mut(),
            0,
            std::ptr::addr_of_mut!(bytes_returned),
            std::ptr::null_mut(),
        )
    };

    if result == FALSE {
        // Fall back to zeroing if FSCTL fails
        punch_hole_fallback(file, offset, len)
    } else {
        Ok(true) // Disk space may be reclaimed (depends on filesystem)
    }
}

/// Fallback implementation: writes zeros (no disk reclamation).
#[cfg(any(
    not(any(target_os = "linux", target_os = "windows")),
    target_os = "linux",
    target_os = "windows"
))]
/// Chunk size for fallback zeroing (64KB).
const FALLBACK_CHUNK_SIZE: usize = 64 * 1024;

fn punch_hole_fallback(file: &File, offset: u64, len: u64) -> io::Result<bool> {
    use std::io::{Seek, SeekFrom, Write};

    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(offset))?;

    // Write zeros in chunks to avoid large allocations
    let zeros = vec![0u8; FALLBACK_CHUNK_SIZE];
    // Reason: `len` represents a byte range within a single file; on supported
    // platforms (64-bit Linux/Windows) usize == u64, so no truncation occurs.
    // On 32-bit targets this function is only reachable for lengths <= usize::MAX.
    #[allow(clippy::cast_possible_truncation)]
    let mut remaining = len as usize;

    while remaining > 0 {
        let to_write = remaining.min(FALLBACK_CHUNK_SIZE);
        file.write_all(&zeros[..to_write])?;
        remaining -= to_write;
    }

    Ok(false) // No disk space reclaimed, only zeroed
}

/// Serializes a flat `id -> offset` index to `path` with fsync.
///
/// The write is in-place (`File::create` truncates first), so this must only
/// target staging paths that are never load-bearing on their own: the
/// `vectors.idx.tmp` sidecar staged BEFORE the data-file swap (covered by
/// [`recover_compaction_artifacts`]) and the `vectors.idx.new` staging file
/// of [`persist_flat_index_atomic`]. Live `vectors.idx` writes must go
/// through [`persist_flat_index_atomic`] — a torn in-place rewrite of
/// `vectors.idx` right after compaction truncated the WAL is unrecoverable.
pub(super) fn persist_flat_index(path: &Path, index: &FxHashMap<u64, usize>) -> io::Result<()> {
    let bytes = postcard::to_allocvec(index).map_err(io::Error::other)?;
    let mut writer = io::BufWriter::new(File::create(path)?);
    writer.write_all(&bytes)?;
    writer.flush()?;
    writer
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)?
        .sync_all()
}

/// Atomically replaces `path` with a freshly serialized flat index.
///
/// Writes to a dedicated `<path>.new` staging file (distinct from the
/// `vectors.idx.tmp` compaction sidecar, so startup recovery never promotes
/// it), fsyncs it, renames it over `path`, then fsyncs the directory so the
/// rename is durable (POSIX). A crash at any point leaves the previous
/// `path` content intact and readable — unlike the former in-place
/// `File::create` rewrite, whose truncate-then-write left a 0-byte/torn
/// `vectors.idx` that bricked `open()` (fatal right after compaction
/// emptied the WAL).
pub(super) fn persist_flat_index_atomic(
    path: &Path,
    index: &FxHashMap<u64, usize>,
) -> io::Result<()> {
    let mut staging = path.as_os_str().to_owned();
    staging.push(".new");
    let staging = std::path::PathBuf::from(staging);

    persist_flat_index(&staging, index)?;
    promote_index_sidecar(&staging, path)?;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty());
    durable_rename_barrier(dir, &[path])
}

/// Renames a fully written index file over `idx_path`.
///
/// On Unix, `rename` atomically replaces the destination. On other platforms
/// the destination is removed first; a crash in between leaves the source in
/// place with the destination missing — startup then either promotes a
/// `vectors.idx.tmp` sidecar (the compaction data swap already committed) or
/// treats the absent `vectors.idx` as empty and rebuilds it from WAL replay.
fn promote_index_sidecar(sidecar: &Path, idx_path: &Path) -> io::Result<()> {
    #[cfg(not(unix))]
    if idx_path.exists() {
        std::fs::remove_file(idx_path)?;
    }
    std::fs::rename(sidecar, idx_path)
}

/// Makes preceding renames durable before the caller truncates the WAL.
///
/// A compaction commit renames the new data/index files into place and then
/// truncates the WAL. If the truncation reaches disk while the renames do not,
/// a power loss leaves an empty WAL beside stale pre-compaction files — silent
/// data loss. This barrier orders the renames before the truncation:
///
/// - **Unix:** fsync the directory, persisting the renames' directory entries.
/// - **Windows/other:** there is no directory fsync, and the data file is
///   memory-mapped so it cannot be reopened for write (`FlushFileBuffers`
///   access-denied). Each *non-mapped* renamed file passed here (the index
///   sidecar) is fsynced instead. Callers must NOT pass a currently-mmapped
///   path. The data-file rename is ordered by NTFS: its record is written to
///   `$LogFile` before the trailing `wal.sync_all()`, and NTFS's write-ahead
///   invariant flushes `$LogFile` up to that point, so the rename is durable
///   before the WAL truncation reaches disk (holds on NTFS; FAT/exFAT data
///   directories are unsupported for durability).
fn durable_rename_barrier(dir: Option<&Path>, renamed: &[&Path]) -> io::Result<()> {
    #[cfg(unix)]
    {
        let _ = renamed;
        match dir {
            Some(d) => File::open(d)?.sync_all(),
            None => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        for path in renamed {
            OpenOptions::new().write(true).open(path)?.sync_all()?;
        }
        Ok(())
    }
}

/// Recovers from interrupted compaction on startup.
///
/// Issue #318: On Windows, `atomic_replace()` uses a two-step rename
/// (dst -> `.bak`, src -> dst). A crash between the two leaves either
/// a `.bak` or `.new` file on disk. This function detects and repairs
/// such states before the mmap file is opened.
///
/// # Recovery Rules
///
/// - `.bak` exists, original missing -> restore from `.bak`
/// - `.bak` exists, original exists -> remove `.bak` (compaction completed)
/// - `vectors.dat.tmp` exists -> the swap (commit point) never happened:
///   remove it and the staged `vectors.idx.tmp`; the old state is intact
/// - only `vectors.idx.tmp` exists -> the swap committed but the crash hit
///   before the index promotion: promote the sidecar to `vectors.idx`
pub fn recover_compaction_artifacts(data_path: &Path) -> io::Result<()> {
    let bak_path = data_path.with_extension("dat.bak");

    // Handle .bak file
    if bak_path.exists() {
        if data_path.exists() {
            // Both exist: previous compaction completed, clean up backup
            std::fs::remove_file(&bak_path)?;
        } else {
            // Only backup exists: compaction crashed after rename-to-backup
            std::fs::rename(&bak_path, data_path)?;
        }
    }

    recover_staged_compaction(data_path)
}

/// Repairs staged compaction files (`vectors.dat.tmp` / `vectors.idx.tmp`).
fn recover_staged_compaction(data_path: &Path) -> io::Result<()> {
    let new_path = data_path.with_extension("dat.tmp");
    let idx_tmp_path = data_path.with_file_name("vectors.idx.tmp");

    if new_path.exists() {
        // Uncommitted compaction: the data swap never happened, so the old
        // dat/idx/WAL triple is authoritative. Drop both staged files.
        std::fs::remove_file(&new_path)?;
        if idx_tmp_path.exists() {
            std::fs::remove_file(&idx_tmp_path)?;
        }
    } else if idx_tmp_path.exists() {
        // The swap committed (vectors.dat.tmp was consumed by the rename) but
        // the crash hit before the index promotion. Finish the commit so the
        // on-disk index matches the compacted layout. The stale WAL is left
        // alone on purpose: replaying it onto the promoted index converges
        // (store records carry the full vector value, deletes replay in
        // order), and the normal replay flow truncates it once the recovered
        // state is durable.
        promote_index_sidecar(&idx_tmp_path, &data_path.with_file_name("vectors.idx"))?;
    }

    Ok(())
}

/// Cross-platform atomic file replacement.
///
/// On Unix, `rename()` atomically replaces the destination.
/// On Windows, `rename()` fails if destination exists, so we use a backup strategy.
fn atomic_replace(src: &Path, dst: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::rename(src, dst)
    }

    #[cfg(windows)]
    {
        // Windows: rename fails if dst exists
        // Strategy: dst -> backup, src -> dst, remove backup
        let backup = dst.with_extension("dat.bak");

        // Remove stale backup if exists
        let _ = std::fs::remove_file(&backup);

        // Move existing dst to backup (if exists)
        if dst.exists() {
            std::fs::rename(dst, &backup)?;
        }

        // Move src to dst
        match std::fs::rename(src, dst) {
            Ok(()) => {
                // Success: remove backup
                let _ = std::fs::remove_file(&backup);
                Ok(())
            }
            Err(e) => {
                // Failed: try to restore backup
                if backup.exists() {
                    let _ = std::fs::rename(&backup, dst);
                }
                Err(e)
            }
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        // Fallback for other platforms
        std::fs::rename(src, dst)
    }
}

/// Compaction configuration and state.
/// EPIC-033/US-004: Updated to use ShardedIndex for reduced lock contention.
pub(super) struct CompactionContext<'a> {
    pub path: &'a Path,
    pub dimension: usize,
    pub index: &'a ShardedIndex,
    pub mmap: &'a RwLock<MmapMut>,
    pub next_offset: &'a AtomicUsize,
    pub wal: &'a RwLock<io::BufWriter<File>>,
    pub initial_size: u64,
    /// Replication-consumer retention state consulted before the WAL is
    /// reclaimed, so compaction never discards a position a consumer still
    /// needs (Requirement 6.3).
    pub watermarks: &'a super::wal_cursor::WalWatermarkRegistry,
}

impl CompactionContext<'_> {
    /// Compacts the storage by rewriting only active vectors.
    ///
    /// This reclaims disk space from deleted vectors by:
    /// 1. Writing all active vectors to a new temporary file
    /// 2. Atomically replacing the old file with the new one
    ///
    /// # TS-CORE-004: Storage Compaction
    ///
    /// This operation is quasi-atomic via `rename()` for crash safety.
    /// The caller holds the storage write lock for the duration, so reads are
    /// blocked while compaction runs (it is not concurrent copy-on-write).
    ///
    /// # Returns
    ///
    /// The number of bytes reclaimed.
    ///
    /// # Errors
    ///
    /// Returns an error if file operations fail.
    pub fn compact(&self) -> io::Result<usize> {
        let vector_size = self.dimension * std::mem::size_of::<f32>();

        // 1. Get current state (EPIC-033/US-004: Use ShardedIndex)
        let active_count = self.index.len();

        if active_count == 0 {
            return Ok(0);
        }

        // Calculate space used vs allocated
        // M-2: Acquire ordering for cross-platform visibility of mmap writes
        let current_offset = self.next_offset.load(Ordering::Acquire);
        let active_size = active_count * vector_size;

        if current_offset <= active_size {
            return Ok(0);
        }

        let bytes_to_reclaim = current_offset - active_size;

        // 2. Create temporary file for compacted data
        let temp_path = self.path.join("vectors.dat.tmp");
        let temp_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temp_path)?;

        // Size the temp file for active vectors
        // Reason: active_size = active_count * vector_size; both are bounded by
        // available memory (usize), so usize -> u64 widens and never truncates.
        #[allow(clippy::cast_possible_truncation)]
        let new_size = (active_size as u64).max(self.initial_size);
        temp_file.set_len(new_size)?;

        // SAFETY: `MmapMut::map_mut` requires a writable file sized for mapping.
        // - Condition 1: `temp_file` was opened read/write and resized via `set_len`.
        // - Condition 2: Mapping length is derived from the file's current size.
        // SAFETY: Compaction copies active bytes through a mutable mmap.
        let mut temp_mmap = unsafe { MmapMut::map_mut(&temp_file)? };

        // 3. Copy active vectors to new file with new offsets
        // EPIC-033/US-004: Snapshot index to HashMap for iteration
        let old_index = self.index.to_hashmap();
        let mmap = self.mmap.read();
        let mut new_index: FxHashMap<u64, usize> = FxHashMap::default();
        new_index.reserve(active_count);

        let mut new_offset = 0usize;
        for (&id, &old_offset) in &old_index {
            // #898: bounds-check the source slice against the live mmap. A
            // corrupt/overflowing index offset must be skipped, not allowed to
            // panic the compaction with an out-of-range slice.
            let Some(src_end) = old_offset.checked_add(vector_size) else {
                tracing::warn!(id, old_offset, "compaction: skipping offset overflow");
                continue;
            };
            if src_end > mmap.len() {
                tracing::warn!(
                    id,
                    old_offset,
                    mmap_len = mmap.len(),
                    "compaction: skipping out-of-bounds vector offset"
                );
                continue;
            }
            let src = &mmap[old_offset..src_end];
            temp_mmap[new_offset..new_offset + vector_size].copy_from_slice(src);
            new_index.insert(id, new_offset);
            new_offset += vector_size;
        }

        drop(mmap);

        // 4. Make the temp file durable (data via msync, metadata via fsync)
        temp_mmap.flush()?;
        drop(temp_mmap);
        temp_file.sync_all()?;
        drop(temp_file);

        // 5. Stage the new index sidecar (vectors.idx.tmp, fsynced) BEFORE the
        // swap, so a crash between the swap and the promotion in step 6 is
        // repaired by `recover_compaction_artifacts` at the next startup.
        let idx_tmp_path = self.path.join("vectors.idx.tmp");
        persist_flat_index(&idx_tmp_path, &new_index)?;

        // 5b. Release the live mapping of the OLD data file BEFORE the swap.
        // Windows refuses to rename/replace a memory-mapped file
        // (ERROR_ACCESS_DENIED), so `atomic_replace` below would fail while the
        // old mapping is held — leaving compaction entirely broken on Windows.
        // Swap in a tiny anonymous placeholder to unmap the file; step 7 remaps
        // the compacted file. Safe because the caller holds `&mut self`
        // (exclusive), so no `VectorSliceGuard` is outstanding to invalidate.
        {
            let placeholder = MmapMut::map_anon(1)?;
            // Dropping the replaced value unmaps the old file-backed mapping,
            // releasing the OS handle so the swap can rename the data file.
            let _ = std::mem::replace(&mut *self.mmap.write(), placeholder);
        }

        // 6. COMMIT POINT: atomically swap the compacted temp file into place.
        // After this succeeds, `vectors.dat` IS the compacted layout.
        let data_path = self.path.join("vectors.dat");
        let swapped = atomic_replace(&temp_path, &data_path);

        // 7. Remap the live data file so storage never stays on the step-5b
        // placeholder. `data_path` is the compacted file if the swap succeeded,
        // otherwise the untouched original.
        let new_data_file = OpenOptions::new().read(true).write(true).open(&data_path)?;
        // SAFETY: `MmapMut::map_mut` requires a writable file sized for mapping.
        // - Condition 1: `new_data_file` is opened read/write after the swap.
        // - Condition 2: File contents are fully materialized by the preceding flush/rename flow.
        // SAFETY: Reloading mmap is required to switch storage to compacted bytes.
        let new_mmap = unsafe { MmapMut::map_mut(&new_data_file)? };
        *self.mmap.write() = new_mmap;

        // 8. Reconcile the in-memory index with what is now on disk. If the swap
        // failed, the original data file is intact and the existing index/offset
        // still describe it, so return the error without touching them.
        swapped?;
        // Swap succeeded: `vectors.dat` is the compacted layout, so the index and
        // offset MUST switch to it. Adopt them BEFORE finalizing, so a failure in
        // `finalize_commit` still leaves the index and the mmap mutually
        // consistent (startup recovery reconciles the durable index/WAL). Issue
        // #316: readers never observe an intermediate empty state (`&mut self`).
        self.index.replace_all(new_index);
        // Reason: Release ordering pairs with the Acquire loads in
        // `should_compact` and `compact` to ensure readers on ARM/weak-memory
        // architectures observe the updated mmap and index before seeing the
        // new offset value.
        self.next_offset.store(new_offset, Ordering::Release);

        // 9. Finalize: promote the staged index sidecar and truncate the obsolete
        // WAL. A failure here is recoverable on the next open and does not desync
        // the already-adopted in-memory index.
        self.finalize_commit(&idx_tmp_path)?;

        Ok(bytes_to_reclaim)
    }

    /// Finalizes a compaction after the data-file swap: promotes the staged
    /// index sidecar, makes the renames durable, and truncates the obsolete WAL.
    /// Runs under the WAL lock so no writer can append between promotion and
    /// truncation. Must be called only after `atomic_replace` has swapped the
    /// compacted `vectors.dat` into place.
    ///
    /// # Crash-safety invariant
    ///
    /// The `vectors.dat.tmp` -> `vectors.dat` rename (done by the caller) is the
    /// single commit point. The compacted file plus the staged index fully
    /// capture every acknowledged write, so once the swap is durable the prior
    /// WAL is obsolete and is truncated. Every intermediate crash point is
    /// recoverable:
    /// - before the swap: startup recovery removes both staged files and the
    ///   old dat/idx/WAL triple is untouched;
    /// - after the swap, before promotion: recovery promotes the sidecar;
    ///   replaying the stale WAL onto the promoted index converges because
    ///   every store record carries the full vector value and deletes replay
    ///   in order;
    /// - after promotion, before truncation: same convergent replay.
    fn finalize_commit(&self, idx_tmp_path: &Path) -> io::Result<()> {
        let mut wal = self.wal.write();

        // Promote the staged index so vectors.idx matches the new layout.
        let idx_path = self.path.join("vectors.idx");
        promote_index_sidecar(idx_tmp_path, &idx_path)?;

        // Make the renames durable before the truncation below can hit disk: a
        // power loss must never persist an empty WAL while rolling back the
        // renames that committed the compaction. On Unix this fsyncs the
        // directory; on Windows the index sidecar is fsynced and the data-file
        // rename is ordered via NTFS $LogFile (see durable_rename_barrier).
        durable_rename_barrier(Some(self.path), &[&idx_path])?;

        // Compaction renders the prior WAL obsolete: reclaim it unless a
        // registered replication consumer's low-watermark still needs a durable
        // position (Requirement 6.3). flush() first so the BufWriter cannot
        // re-emit buffered stale entries; the append fd then writes fresh
        // entries from the new EOF (offset 0).
        wal.flush()?;
        self.reclaim_wal_if_unheld(&wal)
    }

    /// Truncates the WAL to empty unless a registered consumer's low-watermark
    /// still needs a durable position (Requirement 6.3): a registered
    /// replication consumer holds the WAL until its low-watermark passes the
    /// tail, so no needed record is discarded; with no registered consumer this
    /// is the unchanged full truncation.
    fn reclaim_wal_if_unheld(&self, wal: &io::BufWriter<File>) -> io::Result<()> {
        let tail = wal.get_ref().metadata()?.len();
        if super::wal_cursor::watermark_allows_full_truncation(
            self.watermarks.min_watermark(),
            tail,
        ) {
            // The WAL is opened append-only, so its handle lacks FILE_WRITE_DATA
            // and set_len() on it is ACCESS_DENIED on Windows — truncate via a
            // dedicated write handle instead (mirrors wal_replay::truncate_wal),
            // then make the reclaim durable.
            let wal_file = OpenOptions::new()
                .write(true)
                .open(self.path.join("vectors.wal"))?;
            wal_file.set_len(0)?;
            wal_file.sync_all()?;
        }
        Ok(())
    }

    /// Returns the fragmentation ratio (0.0 = no fragmentation, 1.0 = 100% fragmented).
    ///
    /// Use this to decide when to trigger compaction.
    /// A ratio > 0.3 (30% fragmentation) is a good threshold.
    #[must_use]
    pub fn fragmentation_ratio(&self) -> f64 {
        // EPIC-033/US-004: Use ShardedIndex directly
        let active_count = self.index.len();

        if active_count == 0 {
            return 0.0;
        }

        let vector_size = self.dimension * std::mem::size_of::<f32>();
        let active_size = active_count * vector_size;
        // M-2: Acquire ordering for cross-platform visibility
        let current_offset = self.next_offset.load(Ordering::Acquire);

        if current_offset == 0 {
            return 0.0;
        }

        #[allow(clippy::cast_precision_loss)]
        let ratio = 1.0 - (active_size as f64 / current_offset as f64);
        ratio.max(0.0)
    }
}
