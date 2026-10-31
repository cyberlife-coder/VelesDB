//! The rayon pool for parallel work over data a collection lock guards.
//!
//! A thread that holds a collection lock must not wait on the global rayon
//! pool (`crate::lock_audit`, `docs/CONCURRENCY_MODEL.md` "Collection locks
//! and rayon"): a job there that takes the same lock parks behind any queued
//! writer, and the holder then waits for a worker that never frees. The jobs
//! of an application calling a collection from its own rayon code run there
//! too, so no rule on the collection's own jobs could close that cycle.
//!
//! This pool runs only the jobs [`run`] submits. They read through the
//! guards their caller holds and take no collection lock themselves — under
//! `cfg(test)` a collection lock acquired on any rayon worker panics — so a
//! job here never waits on a lock, and a caller holding a guard across
//! [`run`] always gets its workers back.

/// Runs `parallel` on the dedicated pool and waits for it, or runs
/// `sequential` on this thread instead.
///
/// `sequential` runs when this thread is itself a rayon worker: `rayon-core`
/// lets a worker waiting on another pool execute its own pool's jobs
/// meanwhile (`Registry::in_worker_cross`), and one of those may take the
/// lock this thread holds — it would wait on itself. It also runs when the
/// pool cannot be built, which is logged once.
///
/// `parallel`'s parallel iterators run on this pool: rayon keeps nested work
/// on the pool that runs the job.
pub(crate) fn run<R: Send>(
    parallel: impl FnOnce() -> R + Send,
    sequential: impl FnOnce() -> R,
) -> R {
    if rayon::current_thread_index().is_some() {
        return sequential();
    }
    match pool() {
        Some(pool) => pool.install(parallel),
        None => sequential(),
    }
}

/// The pool, built on first use; `None` when it could not be built.
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .thread_name(|i| format!("veles-guarded-{i}"))
            .build()
            .inspect_err(|e| tracing::error!("collection: dedicated rayon pool: {e}"))
            .ok()
    })
    .as_ref()
}
