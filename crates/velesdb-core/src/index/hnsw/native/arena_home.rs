//! Where a graph's f32 arena lives on disk, and what becomes of it after.
//!
//! # When a graph gets a disposable arena, and when it does not
//!
//! The obvious design is to make the arena file *the* stored vectors — map
//! `{basename}.vectors` and skip deserialization entirely. Since #2173 that is
//! exactly what happens, but only when three things hold: the file is format
//! v2, so its payload starts page-aligned where [`FileArena`]'s data region
//! does; the target is little-endian, because `.vectors` is explicitly
//! little-endian while an arena file is native-endian raw memory; and the store
//! reaches `ContiguousVectors::MIN_ARENA_CAPACITY`, below which an arena is
//! sized up to that floor and the file would grow merely by being opened.
//!
//! **A graph that adopted `.vectors` holds no [`ArenaHome`] at all.** That is
//! the whole safety argument for this module after #2173: `Drop` here removes
//! its file unconditionally, so the way a durable store cannot be deleted by
//! accident is that no home is ever constructed for it — not a flag saying not
//! to. [`ArenaHome::claim`] reinforces it from the other side by building
//! `hnsw-{token}.arena` names and nothing else, so the two mechanisms cannot
//! meet even by mistake.
//!
//! Everything outside those three conditions gets a copy of its own instead —
//! a disposable arena file where the storage mode keeps one (SQ8, RaBitQ) and
//! the filesystem can map it, the heap otherwise: v1 files written before #2213, big-endian targets,
//! stores below the capacity floor, and any mapping the filesystem refuses. So
//! does a store that meets all three but must be normalized — a cosine payload
//! written before the engine normalized on insert: normalizing an adopted arena
//! would rewrite `.vectors` on a mere open, so the payload is copied off it
//! first (#2246). A disposable arena is a cache of something already
//! persisted, which is what makes throwing it away free — `.vectors` remains
//! the durable copy, and a reopened collection builds a fresh arena from it.
//!
//! [`sweep_stale`](ArenaHome::sweep_stale) and
//! [`is_arena_file`](ArenaHome::is_arena_file) therefore keep their subject.
//! They exist so a sweep never eats `.vectors`, and disposable arenas still
//! exist to sweep.
//!
//! # What the disposable path costs
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
//! It is also why adoption is preferred wherever it applies — mapping
//! `{basename}.vectors` removes the duplicate entirely: no second copy, no
//! second write, no deletion.
//!
//! An earlier version of this doc called that "a format question, not an
//! architectural one". It was neither statement's fault that both turned out
//! wrong. Aligning the payload was a format change (#2213), but adopting the
//! file also moved *when* vector data becomes durable and *who* may delete it,
//! which is architecture — and the endianness the paragraph named was only one
//! of three conditions, the other two being page alignment and the capacity
//! floor. The reasoning is recorded on #2173 rather than repeated here.
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
