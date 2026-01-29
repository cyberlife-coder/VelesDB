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

    // SAFETY: fallocate is a valid syscall, fd is a valid file descriptor
    let ret = unsafe { libc::fallocate(fd, mode, offset as libc::off_t, len as libc::off_t) };

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
    // SAFETY: Win32 API requires i64 for file offsets. offset and len are typically < i64::MAX
    // on any realistic file system. Saturate to prevent undefined behavior on edge cases.
    #[allow(clippy::cast_possible_wrap)]
    let info = FileZeroDataInformation {
        file_offset: i64::try_from(offset).unwrap_or(i64::MAX),
        beyond_final_zero: i64::try_from(offset.saturating_add(len)).unwrap_or(i64::MAX),
    };

    let mut bytes_returned: u32 = 0;

    // SAFETY: DeviceIoControl is a valid Win32 API call
    let result = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_SET_ZERO_DATA,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of::<FileZeroDataInformation>() as u32,
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
    let mut remaining = len as usize;

    while remaining > 0 {
        let to_write = remaining.min(FALLBACK_CHUNK_SIZE);
        file.write_all(&zeros[..to_write])?;
        remaining -= to_write;
    }

    Ok(false) // No disk space reclaimed, only zeroed
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
    /// Reads remain available during compaction (copy-on-write pattern).
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
        let current_offset = self.next_offset.load(Ordering::Relaxed);
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
        let new_size = (active_size as u64).max(self.initial_size);
        temp_file.set_len(new_size)?;

        // SAFETY: temp_file is a valid, newly created file with set_len() called
        let mut temp_mmap = unsafe { MmapMut::map_mut(&temp_file)? };

        // 3. Copy active vectors to new file with new offsets
        // EPIC-033/US-004: Snapshot index to HashMap for iteration
        let old_index = self.index.to_hashmap();
        let mmap = self.mmap.read();
        let mut new_index: FxHashMap<u64, usize> = FxHashMap::default();
        new_index.reserve(active_count);

        let mut new_offset = 0usize;
        for (&id, &old_offset) in &old_index {
            let src = &mmap[old_offset..old_offset + vector_size];
            temp_mmap[new_offset..new_offset + vector_size].copy_from_slice(src);
            new_index.insert(id, new_offset);
            new_offset += vector_size;
        }

        drop(mmap);

        // 4. Flush temp file
        temp_mmap.flush()?;
        drop(temp_mmap);
        drop(temp_file);

        // 5. Atomic swap: rename temp -> main (cross-platform)
        let data_path = self.path.join("vectors.dat");
        atomic_replace(&temp_path, &data_path)?;

        // 6. Reopen the compacted file
        let new_data_file = OpenOptions::new().read(true).write(true).open(&data_path)?;
        // SAFETY: new_data_file is the compacted file just renamed from temp
        let new_mmap = unsafe { MmapMut::map_mut(&new_data_file)? };

        // 7. Update internal state (EPIC-033/US-004: Clear and repopulate ShardedIndex)
        *self.mmap.write() = new_mmap;
        self.index.clear();
        for (id, offset) in new_index {
            self.index.insert(id, offset);
        }
        self.next_offset.store(new_offset, Ordering::Relaxed);

        // 8. Write compaction marker to WAL
        {
            let mut wal = self.wal.write();
            wal.write_all(&[4u8])?;
            wal.flush()?;
        }

        Ok(bytes_to_reclaim)
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
        let current_offset = self.next_offset.load(Ordering::Relaxed);

        if current_offset == 0 {
            return 0.0;
        }

        #[allow(clippy::cast_precision_loss)]
        let ratio = 1.0 - (active_size as f64 / current_offset as f64);
        ratio.max(0.0)
    }
}
