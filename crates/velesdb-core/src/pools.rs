//! Every rayon pool core submits work to, and the only module that names
//! rayon.
//!
//! Two rules keep a collection lock and a pool from deadlocking
//! (`docs/CONCURRENCY_MODEL.md`, "Collection locks and rayon"), and this
//! module is where each submission is held to them:
//!
//! 1. **No thread holding a collection guard waits on its own pool.** The
//!    parallel helpers below ([`map`] and the others) run on the pool the
//!    calling thread belongs to — the global pool from any thread that is no
//!    rayon worker — and a thread waiting on them executes that pool's other
//!    jobs meanwhile. Each one first calls
//!    [`assert_no_guard_held`](crate::lock_audit::assert_no_guard_held),
//!    which, in every unit test, refuses a caller holding a collection guard.
//! 2. **No rayon worker waits on another pool.** [`on_graph_pool`] and
//!    [`on_guarded_pool`] run work on a dedicated pool whose jobs take no
//!    collection lock and no `HnswIndex::inner`, so a caller holding either
//!    may wait on them — unless the caller is itself a rayon worker: one
//!    waiting on another pool keeps running its own pool's jobs
//!    (`rayon-core`'s `Registry::in_worker_cross`), and one of those, an
//!    application's job that reads the collection behind a queued writer,
//!    would park on the worker the guard holder needs. A worker therefore
//!    gets the sequential form of the work, on its own thread, every time;
//!    no caller can choose otherwise.
//!
//! No other module of the crate names rayon, a thread pool or a parallel
//! iterator: `lock_audit_tests::no_module_but_pools_names_rayon` refuses one
//! that does.

use rayon::prelude::*;

/// Whether this thread is a worker of any rayon pool.
pub(crate) fn on_a_rayon_worker() -> bool {
    rayon::current_thread_index().is_some()
}

// ---------------------------------------------------------------------------
// Dedicated pools
// ---------------------------------------------------------------------------

/// A dedicated pool named `name-{i}`, or `None` (logged) when it cannot be
/// built.
fn dedicated(name: &'static str) -> Option<rayon::ThreadPool> {
    rayon::ThreadPoolBuilder::new()
        .thread_name(move |i| format!("{name}-{i}"))
        .build()
        .inspect_err(|e| tracing::error!("{name}: dedicated rayon pool: {e}"))
        .ok()
}

/// The pool HNSW graph work runs on while its caller holds
/// `HnswIndex::inner` (and, in a merge, a collection guard): the place phase
/// of `HnswIndex::insert_batch_parallel`, the connect phase of
/// `HnswIndex::link_placed` and a vacuum's settle. Its jobs reach the graph's
/// arena, layers and entry point, never `HnswIndex` itself nor a collection
/// lock (\#2343).
fn graph_pool() -> crate::error::Result<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| dedicated("veles-hnsw-graph"))
        .as_ref()
        .ok_or_else(|| {
            crate::error::Error::Internal(
                "the dedicated rayon pool HNSW graph work runs on is unavailable".to_string(),
            )
        })
}

/// Runs `parallel` on the graph pool and waits for it — or, when this
/// thread is a rayon worker, runs `sequential` on it instead (rule 2 of the
/// module documentation).
///
/// # Errors
///
/// Returns an error when the graph pool cannot be built.
pub(crate) fn on_graph_pool<R: Send>(
    parallel: impl FnOnce() -> R + Send,
    sequential: impl FnOnce() -> R,
) -> crate::error::Result<R> {
    if on_a_rayon_worker() {
        return Ok(sequential());
    }
    Ok(graph_pool()?.install(parallel))
}

/// Runs `parallel` on the pool for parallel work over data a collection
/// guard protects, and waits for it — or runs `sequential` on this thread
/// when it is a rayon worker (rule 2 of the module documentation), or when
/// the pool cannot be built.
///
/// Its jobs read through the guards their caller holds and take none, so a
/// caller holding a guard always gets its workers back.
pub(crate) fn on_guarded_pool<R: Send>(
    parallel: impl FnOnce() -> R + Send,
    sequential: impl FnOnce() -> R,
) -> R {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    if on_a_rayon_worker() {
        return sequential();
    }
    match POOL.get_or_init(|| dedicated("veles-guarded")) {
        Some(pool) => pool.install(parallel),
        None => sequential(),
    }
}

// ---------------------------------------------------------------------------
// Parallel helpers: the calling thread's pool, the global one by default
// ---------------------------------------------------------------------------

/// `f` over `items`, in parallel, results in order.
#[track_caller]
pub(crate) fn map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync + Send) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items.par_iter().map(f).collect()
}

/// `f` over `items` and their positions, in parallel, results in order.
#[track_caller]
pub(crate) fn map_enumerated<T: Sync, R: Send>(
    items: &[T],
    f: impl Fn(usize, &T) -> R + Sync + Send,
) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items.par_iter().enumerate().map(|(i, t)| f(i, t)).collect()
}

/// `f` over `items` taken by value, in parallel, results in order.
#[track_caller]
pub(crate) fn map_owned<T: Send, R: Send>(
    items: Vec<T>,
    f: impl Fn(T) -> R + Sync + Send,
) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items.into_par_iter().map(f).collect()
}

/// `f` over `0..count`, in parallel, results in order.
#[track_caller]
pub(crate) fn map_range<R: Send>(count: usize, f: impl Fn(usize) -> R + Sync + Send) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    (0..count).into_par_iter().map(f).collect()
}

/// The `Some` results of `f` over `items` and their positions, in parallel,
/// in order.
#[track_caller]
pub(crate) fn filter_map_enumerated<T: Sync, R: Send>(
    items: &[T],
    f: impl Fn(usize, &T) -> Option<R> + Sync + Send,
) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items
        .par_iter()
        .enumerate()
        .filter_map(|(i, t)| f(i, t))
        .collect()
}

/// `f` over the `size`-long chunks of `items`, in parallel, results in order.
#[track_caller]
pub(crate) fn map_chunks<T: Sync, R: Send>(
    items: &[T],
    size: usize,
    f: impl Fn(&[T]) -> R + Sync + Send,
) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items.par_chunks(size).map(f).collect()
}

/// The `Some` results of `f` over the `size`-long chunks of `items` and
/// their positions, in parallel, in order.
#[track_caller]
pub(crate) fn filter_map_chunks_enumerated<T: Sync, R: Send>(
    items: &[T],
    size: usize,
    f: impl Fn(usize, &[T]) -> Option<R> + Sync + Send,
) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items
        .par_chunks(size)
        .enumerate()
        .filter_map(|(i, chunk)| f(i, chunk))
        .collect()
}

/// The results of `f` over `items`, flattened, in parallel, in order.
#[track_caller]
pub(crate) fn flat_map<T: Sync, R: Send>(
    items: &[T],
    f: impl Fn(&T) -> Vec<R> + Sync + Send,
) -> Vec<R> {
    crate::lock_audit::assert_no_guard_held();
    items.par_iter().flat_map_iter(f).collect()
}

// ---------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------

#[cfg(test)]
thread_local! {
    /// Whether this thread is a worker of a test's stand-in for an
    /// application's own rayon pool.
    static APPLICATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A rayon pool of `threads` workers standing in for an application's own:
/// its jobs may call into a collection, as an application's may, so the
/// lock audit lets them take collection locks.
#[cfg(test)]
pub(crate) fn application_pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("test-application-{i}"))
        .start_handler(|_| APPLICATION.with(|app| app.set(true)))
        .build()
        .expect("test: application pool")
}

/// Whether this thread is a worker of [`application_pool`].
#[cfg(test)]
pub(crate) fn on_application_thread() -> bool {
    APPLICATION.with(std::cell::Cell::get)
}
