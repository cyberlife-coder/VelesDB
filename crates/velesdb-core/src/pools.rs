//! Every rayon pool core submits work to, and the only module that names
//! rayon.

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
