//! Regression test for the blocking-call discipline (audit finding C1).
//!
//! Every handler that calls synchronous, lock-taking or fsync-bearing
//! `velesdb-core` code must run that call on `tokio::task::spawn_blocking`
//! instead of inline on the async runtime. This test runs the server router
//! on a multi-thread runtime with a SINGLE worker thread, fires a wave of
//! fsync-bearing `/query` DML statements (single-row INSERTs — the `/query`
//! handler used to run its core call inline) and, while the wave is still
//! in flight, a trivial `/health` probe.
//!
//! Detection is by ordering, not absolute timing:
//!
//! * Inline (buggy) code: each INSERT (write locks + WAL fsync) occupies
//!   the sole runtime worker for its whole duration, and the probe task —
//!   enqueued after the wave — cannot run until every statement has
//!   completed. The probe therefore observes the entire wave as finished.
//! * Offloaded (fixed) code: every handler parks on `spawn_blocking`, the
//!   worker stays free, and the probe completes in microseconds while most
//!   of the wave is still queued behind the collection write lock on the
//!   blocking pool.
//!
//! A hard watchdog thread (independent of the tokio timer, which itself
//! stalls when the sole worker is blocked) guarantees the test can never
//! hang the suite.

mod common;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::create_test_app_with_state;
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;
use velesdb_core::Point;

/// Rows pre-seeded into the target collection so the INSERTs land in a
/// collection with realistic storage state.
const ROW_COUNT: u64 = 2_000;

/// Batch size for seeding, well under the ingest limits.
const INSERT_BATCH: u64 = 2_000;

/// Number of concurrent fsync-bearing `/query` INSERT statements. Sized so
/// the wave keeps the blocking pool busy for far longer than the probe
/// needs — while an inline (buggy) handler serializes the whole wave on the
/// sole runtime worker ahead of the probe.
const SLOW_REQUESTS: usize = 64;

/// Builds a POST /query request with the given `VelesQL` text.
fn query_request(query: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/query")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "query": query }).to_string()))
        .expect("build /query request")
}

/// Seeds a metadata-only collection with `ROW_COUNT` payload rows.
async fn seed_metadata_collection(state: &Arc<velesdb_server::AppState>) {
    state
        .db
        .create_metadata_collection("bulk")
        .expect("create metadata collection");
    let coll = state
        .db
        .get_any_collection("bulk")
        .expect("collection registered");
    tokio::task::spawn_blocking(move || {
        let mut start = 0u64;
        while start < ROW_COUNT {
            let end = (start + INSERT_BATCH).min(ROW_COUNT);
            let points: Vec<Point> = (start..end)
                .map(|i| {
                    Point::new(
                        i,
                        vec![],
                        Some(json!({
                            "category": format!("cat-{}", i % 50),
                            "value": i,
                            "note": format!("padding payload for row {i} to make the scan cost real"),
                        })),
                    )
                })
                .collect();
            coll.upsert(points).expect("seed upsert");
            start = end;
        }
    })
    .await
    .expect("seeding task");
}

/// Aborts the whole process if the test overruns. This is deliberately a
/// plain OS thread: the tokio timer cannot be trusted here because a
/// starved single-worker runtime stops driving it — which is the very bug
/// under test.
fn spawn_hard_watchdog(done: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(300));
        if !done.load(Ordering::SeqCst) {
            eprintln!(
                "handler_blocking_discipline: hard watchdog fired after 300s — aborting process"
            );
            std::process::exit(1);
        }
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn health_stays_responsive_while_slow_query_scans_run() {
    let done = Arc::new(AtomicBool::new(false));
    spawn_hard_watchdog(Arc::clone(&done));

    let temp_dir = TempDir::new().expect("temp dir");
    let (app, state) = create_test_app_with_state(&temp_dir);
    seed_metadata_collection(&state).await;

    // Fire the wave of slow requests: fsync-bearing single-row INSERT
    // statements. Before the fix these ran inline on the sole async worker.
    let completed = Arc::new(AtomicUsize::new(0));
    let wave_started = Instant::now();
    let mut slow_handles = Vec::with_capacity(SLOW_REQUESTS);
    for i in 0..SLOW_REQUESTS {
        let slow_app = app.clone();
        let counter = Arc::clone(&completed);
        // Each request inserts a distinct row: write locks + WAL fsync on
        // every statement, so caches cannot collapse the wave.
        let id = 1_000_000 + i;
        let sql = format!("INSERT INTO bulk (id, category, value) VALUES ({id}, 'cat-slow', {i})");
        slow_handles.push(tokio::spawn(async move {
            let response = slow_app
                .oneshot(query_request(&sql))
                .await
                .expect("slow /query request");
            counter.fetch_add(1, Ordering::SeqCst);
            response.status()
        }));
    }

    // Trivial fast probe, spawned AFTER the wave. Tasks spawned from this
    // (non-worker) thread land in the runtime's injection queue in spawn
    // order, so the sole worker polls every wave task before the probe:
    //
    // * inline (buggy) handlers run each INSERT to completion inside its
    //   poll, so by the time the probe runs the whole wave has finished;
    // * offloaded (fixed) handlers park on spawn_blocking at their first
    //   poll, so the probe completes while the wave is still in flight.
    let health_app = app.clone();
    let probe_started = Instant::now();
    let health_status = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::spawn(async move {
            health_app
                .oneshot(
                    Request::builder()
                        .uri("/health")
                        .body(Body::empty())
                        .expect("build /health request"),
                )
                .await
                .expect("health request")
                .status()
        }),
    )
    .await
    .expect("watchdog: /health did not complete within 30s — the async worker is starved")
    .expect("health task panicked");
    let health_latency = probe_started.elapsed();
    let completed_at_probe = completed.load(Ordering::SeqCst);

    // Drain the wave and validate every statement actually did the work.
    for handle in slow_handles {
        let status = tokio::time::timeout(Duration::from_secs(120), handle)
            .await
            .expect("watchdog: slow /query did not finish within 120s")
            .expect("slow query task panicked");
        assert_eq!(status, StatusCode::OK, "every /query INSERT must succeed");
    }
    let wave_elapsed = wave_started.elapsed();
    done.store(true, Ordering::SeqCst);

    println!(
        "health latency: {health_latency:?}; wave total: {wave_elapsed:?}; \
         inserts completed when probe returned: {completed_at_probe}/{SLOW_REQUESTS}"
    );

    assert_eq!(health_status, StatusCode::OK, "/health must succeed");
    assert!(
        completed_at_probe < SLOW_REQUESTS,
        "/health only completed after all {SLOW_REQUESTS} fsync-bearing /query \
         INSERTs had finished (health latency {health_latency:?}, wave total \
         {wave_elapsed:?}): the sole runtime worker was blocked by inline core \
         calls in the /query handler instead of parking on spawn_blocking"
    );
    assert!(
        health_latency < Duration::from_secs(5),
        "/health took {health_latency:?} while the /query wave was in flight — \
         the sole runtime worker was blocked by an inline core call"
    );
}
