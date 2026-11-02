//! Unit tests for `runtime.rs`.

use super::build;

#[test]
fn test_explicit_worker_count_reaches_the_runtime() {
    let runtime = build(3).expect("test: runtime");
    assert_eq!(runtime.metrics().num_workers(), 3);
}

/// What `#[tokio::main]` would start: `TOKIO_WORKER_THREADS` when the
/// environment sets it, else one worker per CPU.
fn tokio_main_workers() -> usize {
    std::env::var("TOKIO_WORKER_THREADS").map_or_else(
        |_| std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        |value| {
            value
                .parse()
                .expect("test: TOKIO_WORKER_THREADS is a count")
        },
    )
}

#[test]
fn test_zero_workers_means_what_tokio_main_would_start() {
    let runtime = build(0).expect("test: runtime");
    assert_eq!(runtime.metrics().num_workers(), tokio_main_workers());
}
