//! File-backed storage for a [`ContiguousVectors`] buffer.
//!
//! # Why this exists
//!
//! A quantized index keeps two representations of every vector: the codes it
//! traverses on, and the f32 it re-ranks with. The codes are small; the f32 is
//! not. While that f32 arena is an anonymous allocation it is **un-evictable**,
//! so the index's resident set is `f32 + graph + codes` — larger than the
//! unquantized index it was supposed to shrink (#2112).
//!
//! Backing the arena with a file makes those pages evictable: the kernel can
//! reclaim them under pressure and fault them back in when a re-rank touches
//! them. The resident floor becomes `codes + graph`, which is the whole point
//! of the quantized modes on a small device.
//!
//! # Why the pages are the same bytes the index already persists
//!
//! `{basename}.vectors` was already a raw little-endian f32 blob behind a
//! short header — the arena's own memory layout, written out one value at a
//! time. Mapping that file instead of deserializing it into a fresh
//! allocation removes a full copy from load and makes growth allocation-free:
//! extending the file leaves the existing pages in place, where the heap path
//! has to `memcpy` the whole arena into a bigger block.
//!
//! # Byte order
//!
//! The mapped bytes are native-endian f32: the arena *is* the file, so no
//! conversion happens on either side. That makes an arena file
//! self-consistent on any target but **not portable between targets of
//! different endianness** — it is an index-local cache, not an interchange
//! format, and must be rebuilt rather than copied across such a boundary.
//!
//! Worth stating because the neighbouring `{basename}.vectors` format does
//! the opposite: it converts explicitly (`to_le_bytes`/`from_le_bytes`), so it
//! *is* portable. Anything that later maps that file directly inherits this
//! constraint and must gate on `target_endian`.
//!
//! # Availability
//!
//! `persistence`-only. Without it (WASM, `--no-default-features`) there is no
//! filesystem to map and [`ContiguousVectors`] keeps its anonymous backing;
//! this module is not compiled at all.

use memmap2::{MmapMut, MmapOptions};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

/// Byte offset of the f32 region inside a file-backed arena.
///
/// One page, so the mapped data region starts page-aligned — which also
/// satisfies the arena's 64-byte cache-line preference. The header itself
/// occupies the first bytes of that page; the rest is reserved padding.
///
/// A whole page is deliberate overkill, and the reason is soundness rather
/// than speed. [`ContiguousVectors`] hands out `&[f32]` built with
/// `slice::from_raw_parts`, whose contract requires the pointer to be
/// *properly aligned* — a misaligned arena would be undefined behaviour at the
/// first `get`, long before any SIMD ran. (The SIMD kernels themselves all
/// load unaligned, so they would not have been the ones to complain.) Starting
/// the data region on a page boundary keeps that requirement satisfied by
/// construction instead of by arithmetic that a later edit could break.
///
/// [`ContiguousVectors`]: crate::perf_optimizations::ContiguousVectors
pub(crate) const DATA_OFFSET: usize = 4096;

/// A file mapping that owns the bytes behind a [`ContiguousVectors`] buffer.
///
/// # Invariants
///
/// - `map` is a mapping of at least `DATA_OFFSET + byte_len` bytes of `file`.
/// - The address the mapping lives at is owned by the kernel, not stored
///   inline in this struct, so moving a `FileArena` does **not** move the
///   bytes. That is what makes it sound for `ContiguousVectors` to hold a
///   `NonNull` into the mapping while also owning the `FileArena`.
/// - `path` is kept for diagnostics and for reopening after a grow.
pub(crate) struct FileArena {
    map: MmapMut,
    file: File,
    path: PathBuf,
}

impl FileArena {
    /// Creates (or truncates) `path` and maps `byte_len` bytes of vector data.
    ///
    /// The file is sized to `DATA_OFFSET + byte_len`. A freshly extended file
    /// reads as zeros, which is the same guarantee `alloc_zeroed` gives the
    /// anonymous backing — `insert_at` relies on it when it leaves gaps.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from creating, sizing, or mapping the file.
    pub(crate) fn create(path: &Path, byte_len: usize) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        Self::from_file(file, path.to_path_buf(), byte_len)
    }

    /// Maps an existing file whose data region already holds vector bytes.
    ///
    /// Unlike [`create`](Self::create) this never truncates: the contents are
    /// the arena. The file is only ever grown, to `DATA_OFFSET + byte_len`.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from opening, sizing, or mapping the file.
    pub(crate) fn open(path: &Path, byte_len: usize) -> io::Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        Self::from_file(file, path.to_path_buf(), byte_len)
    }

    /// Sizes `file` to hold `byte_len` data bytes and maps the whole thing.
    fn from_file(file: File, path: PathBuf, byte_len: usize) -> io::Result<Self> {
        let total = Self::total_len(byte_len)?;
        if file.metadata()?.len() < total {
            file.set_len(total)?;
        }
        let map = Self::map(&file, total)?;
        Ok(Self { map, file, path })
    }

    /// `DATA_OFFSET + byte_len`, refusing an overflowing request.
    fn total_len(byte_len: usize) -> io::Result<u64> {
        let total = byte_len.checked_add(DATA_OFFSET).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "vector arena size overflows usize",
            )
        })?;
        u64::try_from(total).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "vector arena size overflows u64",
            )
        })
    }

    /// Maps `total` bytes of `file` for read and write.
    fn map(file: &File, total: u64) -> io::Result<MmapMut> {
        let len = usize::try_from(total).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "vector arena size exceeds this target's address space",
            )
        })?;
        // SAFETY: `MmapMut::map_mut` requires a file that stays valid for the
        // mapping's lifetime and is not concurrently truncated.
        // - Condition 1: `file` is owned by the returned `FileArena`, so it
        //   outlives every use of the mapping.
        // - Condition 2: the file was just sized to at least `len` bytes, and
        //   this type only ever grows it — never truncates.
        // - Condition 3: the file is open for both read and write, which
        //   `map_mut` requires.
        // SAFETY: Map the arena's backing file into the address space.
        let map = unsafe { MmapOptions::new().len(len).map_mut(file) }?;
        Ok(map)
    }

    /// Pointer to the first f32 of the data region.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidData`] if the mapping is somehow
    /// shorter than the header, which would mean the file was truncated
    /// underneath us.
    // Reason: no structural fix exists — producing this pointer is the whole
    // point of the function. The alignment is *proven* rather than assumed: an
    // mmap base is page-aligned by the kernel and `DATA_OFFSET` is a whole
    // page, so the result is 4096-byte aligned against f32's requirement of 4.
    // That proof is load-bearing, not decorative: the slices this pointer
    // feeds are built with `from_raw_parts`, which requires alignment for
    // soundness.
    #[allow(clippy::cast_ptr_alignment)]
    pub(crate) fn data_ptr(&mut self) -> io::Result<NonNull<f32>> {
        if self.map.len() < DATA_OFFSET {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "vector arena mapping is shorter than its header",
            ));
        }
        // SAFETY: `add` requires the result to stay inside the allocation.
        // - Condition 1: the length check above proves `DATA_OFFSET` is within
        //   the mapping.
        // - Condition 2: `as_mut_ptr` returns the mapping's base, which is
        //   non-null and page-aligned, so the offset pointer is non-null and
        //   at least 4096-byte aligned.
        // SAFETY: Address the data region that follows the reserved header.
        let ptr = unsafe { self.map.as_mut_ptr().add(DATA_OFFSET) };
        NonNull::new(ptr.cast::<f32>()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "vector arena mapping produced a null data pointer",
            )
        })
    }

    /// Grows the mapping so the data region holds at least `byte_len` bytes.
    ///
    /// Ordering is load-bearing: the file is extended and the replacement
    /// mapping established **before** the old one is dropped, so a failure at
    /// any step leaves the existing mapping — and therefore every live pointer
    /// into it — untouched. Growing never copies: the pages already written
    /// stay where they are.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from sizing or re-mapping the file. On error the
    /// arena is unchanged and still usable.
    pub(crate) fn grow(&mut self, byte_len: usize) -> io::Result<()> {
        let total = Self::total_len(byte_len)?;
        if self.map.len() as u64 >= total {
            return Ok(());
        }
        self.file.set_len(total)?;
        let fresh = Self::map(&self.file, total)?;
        // Only now is the old mapping released.
        self.map = fresh;
        Ok(())
    }

    /// Flushes dirty pages to the backing file.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from the underlying `msync`.
    pub(crate) fn flush(&self) -> io::Result<()> {
        self.map.flush()
    }

    /// The file this arena is mapped from.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for FileArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileArena")
            .field("path", &self.path)
            .field("mapped_bytes", &self.map.len())
            .finish_non_exhaustive()
    }
}
