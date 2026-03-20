//! Extended tests for [`StreamIngester`] backpressure, shutdown drain,
//! and configuration defaults.

use super::ingester::{BackpressureError, StreamIngester, StreamingConfig};
use crate::collection::types::Collection;
use crate::distance::DistanceMetric;
use crate::point::Point;
use tempfile::TempDir;

/// Helper: create a test collection in a temp directory.
fn test_collection(dim: usize) -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("test_ingester_coll");
    let coll = Collection::create(path, dim, DistanceMetric::Cosine).expect("create collection");
    (dir, coll)
}

/// Helper: create a point with the given id and dimension.
fn make_point(id: u64, dim: usize) -> Point {
    Point {
        id,
        vector: vec![0.1_f32; dim],
        payload: None,
        sparse_vectors: None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Config defaults
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn streaming_config_defaults() {
    let cfg = StreamingConfig::default();
    assert_eq!(cfg.buffer_size, 10_000);
    assert_eq!(cfg.batch_size, 128);
    assert_eq!(cfg.flush_interval_ms, 50);
}

#[test]
fn streaming_config_clone_is_identical() {
    let cfg = StreamingConfig {
        buffer_size: 42,
        batch_size: 7,
        flush_interval_ms: 100,
    };
    let cloned = cfg.clone();
    assert_eq!(cloned.buffer_size, 42);
    assert_eq!(cloned.batch_size, 7);
    assert_eq!(cloned.flush_interval_ms, 100);
}

// ─────────────────────────────────────────────────────────────────────────────
// try_send with backpressure
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ingester_try_send_buffer_full_error_display() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 1,
        batch_size: 1000,
        flush_interval_ms: 60_000,
    };
    let ingester = StreamIngester::new(coll, config);

    // Fill the single-slot channel
    assert!(ingester.try_send(make_point(1, 4)).is_ok());

    let err = ingester.try_send(make_point(2, 4)).unwrap_err();
    assert!(
        matches!(err, BackpressureError::BufferFull),
        "expected BufferFull, got: {err}"
    );
    // Verify Display impl
    let msg = format!("{err}");
    assert!(
        msg.contains("full"),
        "error message should mention 'full': {msg}"
    );

    ingester.shutdown().await;
}

#[tokio::test]
async fn ingester_config_accessor() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 42,
        batch_size: 7,
        flush_interval_ms: 99,
    };
    let ingester = StreamIngester::new(coll, config);

    let c = ingester.config();
    assert_eq!(c.buffer_size, 42);
    assert_eq!(c.batch_size, 7);
    assert_eq!(c.flush_interval_ms, 99);

    ingester.shutdown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Shutdown drains pending items
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ingester_shutdown_drains_all_pending() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 100,
        batch_size: 10_000,        // huge — won't auto-flush by batch
        flush_interval_ms: 60_000, // huge — won't auto-flush by timer
    };
    let coll_clone = coll.clone();
    let ingester = StreamIngester::new(coll, config);

    for i in 1..=5 {
        ingester.try_send(make_point(i, 4)).expect("send ok");
    }

    // Brief yield so drain loop receives points into its channel buffer
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Shutdown flushes everything
    ingester.shutdown().await;

    let results = coll_clone.get(&[1, 2, 3, 4, 5]);
    let found = results.iter().filter(|r| r.is_some()).count();
    assert_eq!(found, 5, "shutdown must flush all pending points");
}

// ─────────────────────────────────────────────────────────────────────────────
// BackpressureError variants
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn backpressure_error_not_configured_display() {
    let err = BackpressureError::NotConfigured;
    let msg = format!("{err}");
    assert!(msg.contains("not configured"));
}

#[test]
fn backpressure_error_drain_task_dead_display() {
    let err = BackpressureError::DrainTaskDead;
    let msg = format!("{err}");
    assert!(msg.contains("dead"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Drain loop integration tests (moved from ingester.rs inline tests)
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_stream_try_send_succeeds_when_capacity_available() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 10,
        batch_size: 100,
        flush_interval_ms: 5000,
    };
    let ingester = StreamIngester::new(coll, config);

    let result = ingester.try_send(make_point(1, 4));
    assert!(
        result.is_ok(),
        "try_send should succeed when channel has capacity"
    );

    ingester.shutdown().await;
}

#[tokio::test]
async fn test_stream_try_send_returns_buffer_full_when_at_capacity() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 2,
        batch_size: 100,
        flush_interval_ms: 60_000,
    };
    let ingester = StreamIngester::new(coll, config);

    assert!(ingester.try_send(make_point(1, 4)).is_ok());
    assert!(ingester.try_send(make_point(2, 4)).is_ok());

    let result = ingester.try_send(make_point(3, 4));
    assert!(result.is_err(), "should return error when buffer full");
    match result.unwrap_err() {
        BackpressureError::BufferFull => {}
        other => panic!("expected BufferFull, got: {other}"),
    }

    ingester.shutdown().await;
}

#[tokio::test]
async fn test_stream_drain_flushes_at_batch_size() {
    let (_dir, coll) = test_collection(4);
    let batch_size = 4;
    let config = StreamingConfig {
        buffer_size: 100,
        batch_size,
        flush_interval_ms: 60_000,
    };
    let coll_clone = coll.clone();
    let ingester = StreamIngester::new(coll, config);

    for i in 0..batch_size {
        ingester
            .try_send(make_point(i as u64 + 1, 4))
            .expect("send should succeed");
    }

    // Poll until batch-size flush completes (max 5s).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let found_count = coll_clone.get(&[1, 2, 3, 4]).iter().flatten().count();
        if found_count == 4 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for batch flush (found {found_count}/4)"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let results = coll_clone.get(&[1, 2, 3, 4]);
    let found_count = results.iter().filter(|r| r.is_some()).count();
    assert_eq!(
        found_count, 4,
        "all {batch_size} points should be flushed via upsert"
    );

    ingester.shutdown().await;
}

#[tokio::test]
async fn test_stream_drain_flushes_partial_batch_after_timeout() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 100,
        batch_size: 100,
        flush_interval_ms: 50,
    };
    let coll_clone = coll.clone();
    let ingester = StreamIngester::new(coll, config);

    ingester.try_send(make_point(1, 4)).expect("send 1");
    ingester.try_send(make_point(2, 4)).expect("send 2");

    // Poll until timer-triggered partial flush completes (max 5s).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let found_count = coll_clone.get(&[1, 2]).iter().flatten().count();
        if found_count == 2 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for partial batch flush (found {found_count}/2)"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let results = coll_clone.get(&[1, 2]);
    let found_count = results.iter().filter(|r| r.is_some()).count();
    assert_eq!(
        found_count, 2,
        "partial batch should be flushed after flush_interval_ms"
    );

    ingester.shutdown().await;
}

#[tokio::test]
async fn test_stream_shutdown_flushes_remaining_points() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 100,
        batch_size: 1000,
        flush_interval_ms: 60_000,
    };
    let coll_clone = coll.clone();
    let ingester = StreamIngester::new(coll, config);

    ingester.try_send(make_point(10, 4)).expect("send");
    ingester.try_send(make_point(11, 4)).expect("send");

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    ingester.shutdown().await;

    let results = coll_clone.get(&[10, 11]);
    let found_count = results.iter().filter(|r| r.is_some()).count();
    assert_eq!(
        found_count, 2,
        "shutdown should flush remaining buffered points"
    );
}

#[tokio::test]
async fn test_stream_delta_drain_loop_routes_to_delta_when_active() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 100,
        batch_size: 4,
        flush_interval_ms: 50,
    };
    let coll_clone = coll.clone();

    coll.delta_buffer.activate();

    let ingester = StreamIngester::new(coll, config);

    for i in 1..=4 {
        ingester
            .try_send(make_point(i, 4))
            .expect("send should succeed");
    }

    // Poll until all 4 points are flushed to storage AND delta buffer (max 5s).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let found = coll_clone.get(&[1, 2, 3, 4]).iter().flatten().count();
        if found == 4 && coll_clone.delta_buffer.len() == 4 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for delta drain flush (storage={found}/4, delta={})",
            coll_clone.delta_buffer.len()
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let results = coll_clone.get(&[1, 2, 3, 4]);
    let found = results.iter().filter(|r| r.is_some()).count();
    assert_eq!(found, 4, "upsert should write all points to storage");

    assert_eq!(
        coll_clone.delta_buffer.len(),
        4,
        "delta buffer should contain the streamed points when active"
    );

    ingester.shutdown().await;
}

#[tokio::test]
#[allow(clippy::cast_possible_truncation)]
async fn test_stream_searchable_immediately() {
    let (_dir, coll) = test_collection(4);
    let config = StreamingConfig {
        buffer_size: 100,
        batch_size: 4,
        flush_interval_ms: 50,
    };
    let coll_clone = coll.clone();
    let ingester = StreamIngester::new(coll, config);

    for i in 1..=4u64 {
        let mut vec = vec![0.0_f32; 4];
        vec[(i as usize - 1) % 4] = 1.0;
        let p = Point {
            id: i,
            vector: vec,
            payload: None,
            sparse_vectors: None,
        };
        ingester.try_send(p).expect("send should succeed");
    }

    // Poll until all 4 points are flushed and searchable (max 5s).
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let found = coll_clone.get(&[1, 2, 3, 4]).iter().flatten().count();
        if found == 4 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for search-ready flush (found {found}/4)"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let query = vec![1.0, 0.0, 0.0, 0.0];
    let results = coll_clone.search(&query, 4).expect("search should succeed");
    assert!(
        !results.is_empty(),
        "inserted points should be searchable after drain"
    );
    assert_eq!(results[0].point.id, 1, "closest match should be id=1");

    ingester.shutdown().await;
}

#[tokio::test]
#[allow(clippy::cast_precision_loss)]
async fn test_stream_delta_rebuild_no_data_loss() {
    let (_dir, coll) = test_collection(4);
    let initial_points: Vec<Point> = (1..=5u64)
        .map(|i| {
            let mut vec = vec![0.0_f32; 4];
            vec[0] = i as f32;
            Point {
                id: i,
                vector: vec,
                payload: None,
                sparse_vectors: None,
            }
        })
        .collect();
    coll.upsert(initial_points).expect("upsert initial points");

    coll.delta_buffer.activate();
    assert!(coll.delta_buffer.is_active());

    let config = StreamingConfig {
        buffer_size: 100,
        batch_size: 4,
        flush_interval_ms: 50,
    };
    let coll_clone = coll.clone();
    let ingester = StreamIngester::new(coll, config);

    for i in 6..=10u64 {
        let mut vec = vec![0.0_f32; 4];
        vec[0] = i as f32;
        let p = Point {
            id: i,
            vector: vec,
            payload: None,
            sparse_vectors: None,
        };
        ingester.try_send(p).expect("send should succeed");
    }

    // Poll until all 5 streamed points are visible (max 5s), avoiding
    // a fixed sleep that is fragile under load.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let count = coll_clone.get(&[6, 7, 8, 9, 10]).iter().flatten().count();
        if count == 5 {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for streamed points to flush (found {count}/5)"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    let query = vec![10.0, 0.0, 0.0, 0.0];
    let results = coll_clone
        .search_ids(&query, 10)
        .expect("search_ids should succeed");
    let found_ids: std::collections::HashSet<u64> = results.iter().map(|sr| sr.id).collect();

    for id in 1..=10 {
        assert!(
            found_ids.contains(&id),
            "point id={id} should be in search results"
        );
    }

    let drained = coll_clone.delta_buffer.deactivate_and_drain();
    assert!(!coll_clone.delta_buffer.is_active());
    assert_eq!(drained.len(), 5, "delta should have had 5 entries");

    ingester.shutdown().await;
}
