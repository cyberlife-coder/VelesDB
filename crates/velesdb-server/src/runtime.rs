//! The Tokio runtime the server runs on, sized by `[server] workers`.
//!
//! `main` used to be `#[tokio::main]`, whose multi-threaded runtime always
//! starts one worker per CPU. It builds this one instead, so the setting can
//! reach the runtime before anything runs on it.

/// Builds a multi-threaded runtime with every driver enabled — what
/// `#[tokio::main]` builds — with `workers` worker threads, or one per CPU
/// (Tokio's own default) when `workers` is `0`.
///
/// # Errors
///
/// Returns the I/O error Tokio reports when the runtime cannot start.
pub fn build(workers: usize) -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if workers > 0 {
        builder.worker_threads(workers);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::build;

    #[test]
    fn test_explicit_worker_count_reaches_the_runtime() {
        let runtime = build(3).expect("test: runtime");
        assert_eq!(runtime.metrics().num_workers(), 3);
    }

    #[test]
    fn test_zero_workers_means_one_per_cpu_like_tokio_main() {
        let runtime = build(0).expect("test: runtime");
        let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
        assert_eq!(runtime.metrics().num_workers(), cpus);
    }
}
