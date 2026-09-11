//! The arena that IS the durable `.vectors`: mapping it on load, rewriting only
//! its header on dump, and copying it to the heap when a load must modify it.
//!
//! Split out of `graph_io.rs` when the unit-norm flag and the open-never-writes
//! copy pushed that file past its 1000-line budget (#2246). The seam is what
//! these three share and nothing else in the file does: each one exists because
//! the arena and the store can be the same bytes (#2173).

use super::{DistanceEngine, NativeHnsw, VECTORS_FORMAT_VERSION};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

impl<D: DistanceEngine + Send + Sync> NativeHnsw<D> {
    /// Rewrites the header of a `.vectors` file the arena is mapped from.
    ///
    /// The payload is already in the file — it *is* the arena — so nothing but
    /// the header needs writing. `File::create` would truncate the very bytes
    /// the live mapping still points at, a SIGBUS on the next read rather than
    /// a slow path, so the file is opened for writing without truncation. The
    /// header region is the first [`VECTORS_V2_DATA_OFFSET`](super::VECTORS_V2_DATA_OFFSET) bytes and the
    /// mapping starts after it, so the two never address the same bytes.
    ///
    /// Pages before header, deliberately. A header claiming more vectors than
    /// the file holds is the one state a reader cannot detect: it validates the
    /// declared payload against the file length, and a stale-but-smaller count
    /// simply reads fewer vectors. The generation stamp written after this call
    /// is still what commits the set.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the flush, the open or the header write fails.
    pub(super) fn rewrite_adopted_vectors_header(
        vectors: &crate::perf_optimizations::ContiguousVectors,
        vectors_path: &Path,
        count: u64,
        dimension: u32,
        unit_norm: bool,
    ) -> std::io::Result<()> {
        vectors.flush_backing().map_err(std::io::Error::other)?;
        let mut file = OpenOptions::new().write(true).open(vectors_path)?;
        Self::write_vectors_header(&mut file, count, dimension, unit_norm)?;
        file.flush()
    }

    /// A copy of `storage`, slot for slot, in the arena this graph would have
    /// had without adoption: file-backed under `home` when the storage mode
    /// keeps one (SQ8, `RaBitQ`), the heap otherwise.
    ///
    /// For the one case an adopted arena must stop being the file: a payload
    /// the load has to modify, which must never reach the durable `.vectors`.
    /// Made under the raised allocation ceiling `read_vector_data` uses, for
    /// the same reason: a legitimately persisted payload must reload whatever
    /// the process-wide backstop, and copying it is no different.
    ///
    /// # Errors
    ///
    /// Returns `io::Error` if the arena cannot be allocated or mapped.
    pub(super) fn detached_copy(
        storage: &crate::perf_optimizations::ContiguousVectors,
        home: Option<&crate::index::hnsw::native::arena_home::ArenaHome>,
    ) -> std::io::Result<crate::perf_optimizations::ContiguousVectors> {
        let (dimension, len) = (storage.dimension(), storage.len());
        let min_bytes = len
            .checked_mul(dimension)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| std::io::Error::other("vector payload size overflows usize"))?;
        crate::alloc_guard::with_min_alloc_byte_limit(min_bytes, || {
            let mut copy = Self::new_arena(home, dimension, len).map_err(std::io::Error::other)?;
            for i in 0..len {
                if let Some(vector) = storage.get(i) {
                    copy.insert_at(i, vector).map_err(std::io::Error::other)?;
                }
            }
            Ok(copy)
        })
    }

    /// Maps `path` as the graph's arena when the file can serve as one (#2173).
    ///
    /// `None` means the caller must fall back to reading the payload into a
    /// separate arena. Three conditions have to hold, each for its own reason:
    ///
    /// - **The payload must be page-aligned**, which only v2 guarantees. A v1
    ///   payload starts at byte 16, where `FileArena`'s data region cannot.
    /// - **The target's byte order must be the payload's.** `.vectors` is
    ///   explicitly little-endian; mapping it on a big-endian target would
    ///   reinterpret every float rather than convert it.
    /// - **The arena must keep its exact capacity**
    ///   ([`ContiguousVectors::keeps_exact_capacity`]).
    ///   Below that floor an arena is sized up to it and the file grows to
    ///   match, so adoption would *write*. Opening a collection must never
    ///   write to it — `velesdb-memory`'s migration resume proves the source
    ///   store unchanged by hashing these files, so a store that grew on open
    ///   would make a correct resume look like a corrupted one. What adoption
    ///   saves below the floor is negligible anyway.
    ///
    /// A refused mapping is a warning, never an error: a mapped arena is an
    /// optimisation, not a requirement. That is the rule `new_arena` states for
    /// the disposable arena, for the same reason — this must never stop a
    /// collection from opening that opened fine before the feature existed.
    ///
    /// [`ContiguousVectors::keeps_exact_capacity`]: crate::perf_optimizations::ContiguousVectors::keeps_exact_capacity
    pub(super) fn adopt_durable_file(
        path: &Path,
        version: u32,
        count: usize,
        dimension: usize,
    ) -> Option<crate::perf_optimizations::ContiguousVectors> {
        use crate::perf_optimizations::ContiguousVectors;

        if version != VECTORS_FORMAT_VERSION
            || !cfg!(target_endian = "little")
            || !ContiguousVectors::keeps_exact_capacity(count)
        {
            return None;
        }

        // Capacity is the count: the file holds exactly the payload its header
        // declares, and asking for a larger arena is what would extend it.
        match ContiguousVectors::open_file_backed(path, dimension, count, count) {
            Ok(storage) => Some(storage),
            Err(e) => {
                tracing::warn!(
                    "{path:?} could not be adopted as the vector arena ({e}); \
                     reading it into a separate arena instead, which costs a copy, \
                     not correctness"
                );
                None
            }
        }
    }
}
