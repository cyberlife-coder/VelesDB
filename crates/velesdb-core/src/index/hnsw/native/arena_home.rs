//! Where a graph's f32 arena lives on disk, and what becomes of it after.
//!
//! # Why the arena is disposable rather than persistent
//!
//! The obvious design is to make the arena file *the* stored vectors — map
//! `{basename}.vectors` and skip deserialization entirely. Two facts rule
//! that out for now, and both were found by reading the code rather than
//! assumed:
//!
//! 1. **Vacuum makes two live indexes on purpose.** `build_vacuum_replacement`
//!    constructs a whole replacement graph while the old one is still serving
//!    reads, then swaps. A single well-known arena path would put both of them
//!    on the same bytes, and [`FileArena`]'s exclusive lock — which exists
//!    because two mappings of one file are two `&mut [f32]` aliases — would
//!    refuse the second. The lock would be doing its job; the design would be
//!    wrong.
//! 2. **`.vectors` is portable and the arena is not.** `.vectors` converts
//!    explicitly through `to_le_bytes`; an arena file is native-endian raw
//!    memory. Merging them trades a portable on-disk format for one that
//!    cannot cross an endianness boundary — a real cost, for a load-time win
//!    that #2112 never asked for.
//!
//! So each live graph gets its **own** arena file, and deletes it when it
//! drops. The arena is a cache of something already persisted, which is what
//! makes throwing it away free: `.vectors` remains the durable copy, and a
//! reopened collection simply builds a fresh arena from it.
//!
//! # What this costs, and why the end state is different
//!
//! Being a second copy is not free, and the cost is write volume rather than
//! space. Loading a collection writes every vector through the mapping, so
//! those pages are dirty; the kernel writes them back on its own schedule,
//! and then the file is deleted when the graph drops. The vector data is
//! therefore written to disk roughly **twice per collection lifetime** —
//! once into `.vectors`, once into an arena nobody will ever read again.
//!
//! Nothing here should try to hurry that along. An explicit `msync` would
//! only force the useless half of that writeback to happen sooner, spending
//! flash endurance to persist bytes already durable in `.vectors`; the pages
//! become reclaimable either way once the kernel has written them, which is
//! the property the resident-set argument actually needs.
//!
//! The trade is deliberate: on a memory-constrained device, spending write
//! bandwidth to move the f32 arena out of the resident set is usually worth
//! it, since that arena is the single largest thing a quantized index holds.
//! But it *is* a trade, and it is the reason the end state is not this
//! design. Mapping `{basename}.vectors` itself removes the duplicate
//! entirely — no second copy, no second write, no deletion — and the only
//! thing standing in the way is that the arena is native-endian where
//! `.vectors` is explicitly little-endian. That is a format question, not an
//! architectural one, and it is tracked separately.
//!
//! [`FileArena`]: crate::contiguous_file_arena::FileArena

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Hands out a distinct arena file to every graph in this process.
///
/// Two live graphs over one collection is a normal state, not an error — see
/// the module doc — so uniqueness has to come from somewhere other than the
/// collection's identity. A counter is enough: the file is deleted on drop
/// and never read by anyone but its own graph, so the value carries no
/// meaning beyond "not the same as the others".
static NEXT_ARENA_TOKEN: AtomicU64 = AtomicU64::new(0);

/// Owns one graph's arena file and removes it when the graph goes away.
///
/// # Drop order
///
/// The mapping must be released before the file is unlinked, so whatever
/// holds an `ArenaHome` must declare it **after** the field holding the
/// [`ContiguousVectors`] it belongs to. Rust drops fields in declaration
/// order, so that ordering is the whole mechanism — the same technique
/// `HnswIndex` already uses for `inner` and `io_holder`.
///
/// Unlinking a mapped file succeeds on Unix and fails on Windows, which is
/// the other reason the order is not merely tidy.
///
/// [`ContiguousVectors`]: crate::perf_optimizations::ContiguousVectors
#[derive(Debug)]
pub(crate) struct ArenaHome {
    path: PathBuf,
}

impl ArenaHome {
    /// Claims a fresh arena path inside `dir`.
    ///
    /// Creates no file: that is [`ContiguousVectors::new_file_backed`]'s job,
    /// and it may never happen if the graph takes no vectors.
    ///
    /// [`ContiguousVectors::new_file_backed`]: crate::perf_optimizations::ContiguousVectors::new_file_backed
    pub(crate) fn claim(dir: &Path) -> Self {
        let token = NEXT_ARENA_TOKEN.fetch_add(1, Ordering::Relaxed);
        Self {
            path: dir.join(format!("{ARENA_PREFIX}{token}.{ARENA_EXTENSION}")),
        }
    }

    /// The file this graph's arena should occupy.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Removes arena files left behind by a process that did not drop them.
    ///
    /// A crash or a kill skips [`Drop`], so a collection directory can carry
    /// arenas from a previous run. They are unreadable to anyone — the token
    /// that named them is gone — so they are pure waste, and sweeping is safe
    /// precisely because nothing else may be running: the database holds an
    /// exclusive lock on its directory for as long as it is open.
    ///
    /// # Call it before any arena is claimed, and only then
    ///
    /// This deletes *every* file it recognises, and it cannot tell a live
    /// arena from an abandoned one — the token in the name is meaningless
    /// outside the process that issued it. Calling it while a graph holds an
    /// arena in `dir` unlinks that graph's file: on Unix the mapping survives
    /// but the file is gone, on Windows the delete fails. So it belongs at
    /// collection open/create, before any graph exists, and nowhere else.
    ///
    /// Best-effort by construction. A file that cannot be removed is a
    /// diagnostic, not a reason to refuse to open a collection whose real
    /// data is intact.
    pub(crate) fn sweep_stale(dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if Self::is_arena_file(&path) {
                if let Err(e) = std::fs::remove_file(&path) {
                    tracing::debug!("could not sweep stale vector arena {path:?}: {e}");
                }
            }
        }
    }

    /// Whether `path` names a file this module owns.
    ///
    /// The one definition of that question. [`sweep_stale`](Self::sweep_stale)
    /// deletes everything it accepts, so a second, drifting copy of this
    /// predicate is how a sweep starts eating `.vectors`.
    ///
    /// Matches on the extension rather than a string suffix so a
    /// case-insensitive filesystem cannot hide a file from the sweep that a
    /// case-sensitive one would have removed.
    pub(in crate::index::hnsw) fn is_arena_file(path: &Path) -> bool {
        let named = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(ARENA_PREFIX));
        named
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ARENA_EXTENSION))
    }
}

/// Marks a file as this module's to create and to delete.
///
/// Both halves are load-bearing: [`ArenaHome::sweep_stale`] deletes every
/// file that matches, so the pattern must not be able to name anything a
/// collection actually needs. `.vectors`, `.graph` and the payload log all
/// fail it.
const ARENA_PREFIX: &str = "hnsw-";
/// See [`ARENA_PREFIX`].
const ARENA_EXTENSION: &str = "arena";

impl Drop for ArenaHome {
    fn drop(&mut self) {
        // Missing is the expected case for a graph that never took a vector,
        // so absence is not worth a log line.
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::debug!("could not remove vector arena {:?}: {e}", self.path),
        }
    }
}
