//! Regression tests for the async-runtime starvation fix in `VelesDbState`.
//!
//! Two properties are locked in at the state layer (no Tauri app needed):
//!
//! 1. `run_db` executes core operations on the blocking pool, so a slow
//!    operation never stalls async runtime workers: with the runtime
//!    constrained to a single worker thread, a fast command completes while a
//!    slow one is still in flight.
//! 2. Neither `run_db` nor `with_db` holds the internal state lock across the
//!    operation: while a slow operation is paused mid-flight, `open()` (which
//!    takes the state's write lock) and a second `with_db` both succeed
//!    immediately instead of blocking behind a held read guard.
//!
//! Every wait is bounded by a watchdog timeout so a regression fails the test
//! instead of hanging it.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri_plugin_velesdb::VelesDbState;

/// Upper bound for any single wait; generous so slow CI never flakes.
const WATCHDOG: Duration = Duration::from_secs(30);

/// Bound within which the fast operation must finish while the slow one is
/// still in flight. Generous (the fast op takes microseconds; the slow op is
/// gated on a channel, not a timer, so there is no race to win).
const FAST_BOUND: Duration = Duration::from_secs(10);

fn opened_state(dir: &std::path::Path) -> Arc<VelesDbState> {
    let state = Arc::new(VelesDbState::new(dir.to_path_buf()));
    // Warm the lazy open so measurements below exclude first-open cost.
    state.open().expect("open database");
    state
}

/// With a single-worker async runtime, a fast `run_db` call must complete
/// while a slow `run_db` call is still executing. Before the fix, the slow
/// synchronous core call ran inline on the (only) runtime worker, so the fast
/// command could not even start until the slow one finished.
#[test]
fn fast_command_completes_while_slow_operation_is_in_flight() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let state = opened_state(dir.path());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("build single-worker runtime");

    runtime.block_on(async move {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        // Slow operation: signals once it is running, then blocks until
        // released. It runs on the blocking pool, not on the async worker.
        let slow_state = Arc::clone(&state);
        let slow = tokio::spawn(async move {
            slow_state
                .run_db(move |_db| {
                    let _ = started_tx.send(());
                    let _ = release_rx.recv_timeout(WATCHDOG);
                    Ok(())
                })
                .await
        });

        // Watchdog: the slow operation must actually be in flight.
        tokio::time::timeout(WATCHDOG, started_rx)
            .await
            .expect("watchdog: slow operation never started")
            .expect("slow operation dropped its start signal");

        // Fast operation on the same single-worker runtime, while the slow
        // one is still blocked inside its core call.
        let fast_started = Instant::now();
        let collections = tokio::time::timeout(
            FAST_BOUND,
            state.run_db(|db| Ok(db.list_collections().len())),
        )
        .await
        .expect("fast command starved: runtime worker blocked by slow operation")
        .expect("fast command failed");
        assert_eq!(collections, 0);
        assert!(
            fast_started.elapsed() < FAST_BOUND,
            "fast command took {:?}, exceeding the {FAST_BOUND:?} bound",
            fast_started.elapsed()
        );

        // Release the slow operation and check it finishes cleanly.
        release_tx.send(()).expect("release slow operation");
        tokio::time::timeout(WATCHDOG, slow)
            .await
            .expect("watchdog: slow operation never finished")
            .expect("join slow task")
            .expect("slow operation failed");
    });
}

/// The state lock must not be held while an operation runs: with a `with_db`
/// operation paused mid-flight, `open()` (write lock) and a second `with_db`
/// (read lock) must both return promptly. Before the fix, `with_db` held the
/// state's read guard across the whole operation, so `open()` would block
/// until the operation finished.
#[test]
fn state_lock_is_not_held_across_with_db_operation() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let state = opened_state(dir.path());

    let (started_tx, started_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();

    // Slow operation paused mid-flight on a plain thread (sync API).
    let slow_state = Arc::clone(&state);
    let slow = std::thread::spawn(move || {
        slow_state.with_db(move |_db| {
            let _ = started_tx.send(());
            let _ = release_rx.recv_timeout(WATCHDOG);
            Ok(())
        })
    });
    started_rx
        .recv_timeout(WATCHDOG)
        .expect("watchdog: slow operation never started");

    // open() takes the state's write lock; run it on a helper thread with a
    // watchdog so a held read guard fails the test instead of hanging it.
    let (open_done_tx, open_done_rx) = mpsc::channel();
    let open_state = Arc::clone(&state);
    let opener = std::thread::spawn(move || {
        let result = open_state.open();
        let _ = open_done_tx.send(result);
    });
    open_done_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("state lock still held during with_db operation: open() blocked")
        .expect("concurrent open failed");
    opener.join().expect("join opener thread");

    // A second with_db-style access also succeeds while the slow op runs.
    let collections = state
        .with_db(|db| Ok(db.list_collections().len()))
        .expect("concurrent with_db failed");
    assert_eq!(collections, 0);

    release_tx.send(()).expect("release slow operation");
    slow.join()
        .expect("join slow thread")
        .expect("slow with_db failed");
}
