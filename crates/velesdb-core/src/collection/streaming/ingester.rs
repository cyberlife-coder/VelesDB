//! StreamIngester: bounded-channel ingestion with micro-batch drain.

use crate::collection::types::Collection;
use crate::point::Point;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Notify;

/// Configuration for the streaming ingestion pipeline.
///
/// Controls channel capacity, micro-batch sizing, and flush timing.
///
/// # Defaults
///
/// | Parameter          | Default  |
/// |--------------------|----------|
/// | `buffer_size`      | 10 000   |
/// | `batch_size`       | 128      |
/// | `flush_interval_ms`| 50       |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamingConfig {
    /// Capacity of the bounded mpsc channel (backpressure threshold).
    pub buffer_size: usize,

    /// Number of points that trigger an immediate micro-batch flush.
    pub batch_size: usize,

    /// Maximum time (ms) before a partial batch is flushed.
    pub flush_interval_ms: u64,
}

impl Default for StreamingConfig {
    fn default() -> Self {
        Self {
            buffer_size: 10_000,
            batch_size: 128,
            flush_interval_ms: 50,
        }
    }
}

/// Internal write mode discriminator (not exposed to users).
///
/// Distinguishes between API-driven writes (synchronous upsert) and
/// streaming-driven writes (micro-batch drain).
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteMode {
    /// Standard synchronous API upsert.
    Api,
    /// Streaming micro-batch drain.
    Streaming,
}

/// Error returned when the streaming channel is at capacity.
#[derive(Debug)]
pub enum BackpressureError {
    /// The ingestion buffer is full; the caller should retry after a short delay.
    BufferFull,
    /// Streaming is not configured on this collection.
    NotConfigured,
}

impl std::fmt::Display for BackpressureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferFull => write!(f, "streaming buffer is full (backpressure)"),
            Self::NotConfigured => write!(f, "streaming is not configured on this collection"),
        }
    }
}

impl std::error::Error for BackpressureError {}

/// Streaming ingestion handle for a single collection.
///
/// Owns a bounded mpsc sender and a background drain task. Points sent via
/// [`try_send`](Self::try_send) are accumulated into micro-batches and flushed
/// to the collection's existing `upsert` pipeline.
///
/// # Shutdown
///
/// Call [`shutdown`](Self::shutdown) to gracefully drain remaining points.
/// If dropped without shutdown, the drain task is aborted (points in the
/// channel may be lost).
pub struct StreamIngester {
    sender: mpsc::Sender<Point>,
    config: StreamingConfig,
    drain_handle: Option<tokio::task::JoinHandle<()>>,
    shutdown: Arc<Notify>,
}

impl StreamIngester {
    /// Creates a new streaming ingester for the given collection.
    ///
    /// Spawns a background drain task that accumulates points and flushes
    /// micro-batches via `Collection::upsert`.
    #[must_use]
    pub fn new(collection: Collection, config: StreamingConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.buffer_size);
        let shutdown = Arc::new(Notify::new());

        let drain_handle = tokio::spawn(drain_loop(
            collection,
            rx,
            config.batch_size,
            config.flush_interval_ms,
            Arc::clone(&shutdown),
        ));

        Self {
            sender: tx,
            config,
            drain_handle: Some(drain_handle),
            shutdown,
        }
    }

    /// Attempts to send a point into the streaming channel.
    ///
    /// Returns immediately. If the channel is at capacity, returns
    /// [`BackpressureError::BufferFull`].
    ///
    /// # Errors
    ///
    /// Returns [`BackpressureError::BufferFull`] when the bounded channel is full.
    pub fn try_send(&self, point: Point) -> Result<(), BackpressureError> {
        self.sender.try_send(point).map_err(|_| {
            // Both Full and Closed map to BufferFull: a closed channel means the
            // drain task exited, which is functionally equivalent to being full.
            BackpressureError::BufferFull
        })
    }

    /// Returns a reference to the streaming configuration.
    #[must_use]
    pub fn config(&self) -> &StreamingConfig {
        &self.config
    }

    /// Gracefully shuts down the ingester, flushing any remaining buffered points.
    ///
    /// This notifies the drain loop to exit and awaits its completion.
    pub async fn shutdown(mut self) {
        self.shutdown.notify_one();
        if let Some(handle) = self.drain_handle.take() {
            // Ignore JoinError — the drain loop should not panic.
            let _ = handle.await;
        }
    }
}

impl Drop for StreamIngester {
    fn drop(&mut self) {
        // Abort the drain task to prevent orphaned background tasks.
        // For graceful shutdown with flush, call `shutdown()` before dropping.
        if let Some(handle) = self.drain_handle.take() {
            handle.abort();
        }
    }
}

/// Background drain loop that accumulates points and flushes micro-batches.
///
/// Uses `tokio::select!` with three branches:
/// 1. Shutdown notification — flush remaining batch and exit.
/// 2. Timer tick — flush partial batch if non-empty.
/// 3. Channel receive — push to batch; flush when `batch_size` reached.
async fn drain_loop(
    collection: Collection,
    mut rx: mpsc::Receiver<Point>,
    batch_size: usize,
    flush_interval_ms: u64,
    shutdown: Arc<Notify>,
) {
    let mut batch: Vec<Point> = Vec::with_capacity(batch_size);
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(flush_interval_ms));
    // The first tick completes immediately; consume it.
    interval.tick().await;

    loop {
        tokio::select! {
            // Branch 1: shutdown signal
            () = shutdown.notified() => {
                // Drain any remaining items from the channel.
                while let Ok(point) = rx.try_recv() {
                    batch.push(point);
                }
                if !batch.is_empty() {
                    flush_batch(&collection, &mut batch).await;
                }
                break;
            }

            // Branch 2: timer tick — flush partial batch
            _ = interval.tick() => {
                if !batch.is_empty() {
                    flush_batch(&collection, &mut batch).await;
                }
            }

            // Branch 3: receive point from channel
            msg = rx.recv() => {
                if let Some(point) = msg {
                    batch.push(point);
                    if batch.len() >= batch_size {
                        flush_batch(&collection, &mut batch).await;
                        // Reset the interval so the timer doesn't fire
                        // immediately after a batch-size flush.
                        interval.reset();
                    }
                } else {
                    // Channel closed (all senders dropped).
                    if !batch.is_empty() {
                        flush_batch(&collection, &mut batch).await;
                    }
                    break;
                }
            }
        }
    }
}

/// Flushes the accumulated batch via the collection's existing upsert pipeline.
///
/// Runs the blocking upsert on Tokio's blocking thread pool to avoid stalling
/// the async runtime. If the delta buffer is active (HNSW rebuild in progress),
/// also pushes the batch vectors into the delta buffer for immediate searchability.
async fn flush_batch(collection: &Collection, batch: &mut Vec<Point>) {
    let points: Vec<Point> = std::mem::take(batch);

    // Snapshot vectors for delta buffer before moving points into upsert.
    // Only allocate if delta is active (common case: delta is inactive).
    let delta_entries: Vec<(u64, Vec<f32>)> = if collection.delta_buffer.is_active() {
        points.iter().map(|p| (p.id, p.vector.clone())).collect()
    } else {
        Vec::new()
    };

    let coll = collection.clone();
    // spawn_blocking wraps the synchronous upsert call (which acquires
    // multiple RwLocks and does mmap I/O) to prevent blocking the async runtime.
    let result = tokio::task::spawn_blocking(move || coll.upsert(points)).await;
    match result {
        Ok(Ok(())) => {
            // After successful upsert, push to delta buffer if active.
            // The upsert wrote to storage+WAL; delta is an additional runtime
            // copy so search can find these vectors before HNSW is rebuilt.
            if !delta_entries.is_empty() {
                collection.delta_buffer.extend(delta_entries);
            }
        }
        Ok(Err(e)) => {
            tracing::error!("Streaming drain flush failed: {e}");
        }
        Err(e) => {
            tracing::error!("Streaming drain task panicked: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distance::DistanceMetric;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Helper: create a test collection in a temp directory.
    fn test_collection(dim: usize) -> (TempDir, Collection) {
        let dir = TempDir::new().expect("tempdir");
        let path = dir.path().join("test_stream_coll");
        let coll =
            Collection::create(path, dim, DistanceMetric::Cosine).expect("create collection");
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
            batch_size: 100, // Large batch_size so drain doesn't flush quickly
            flush_interval_ms: 60_000, // Very long interval to avoid timer flush
        };
        let ingester = StreamIngester::new(coll, config);

        // Fill the channel (capacity = 2)
        assert!(ingester.try_send(make_point(1, 4)).is_ok());
        assert!(ingester.try_send(make_point(2, 4)).is_ok());

        // Third send should fail with BufferFull
        let result = ingester.try_send(make_point(3, 4));
        assert!(result.is_err(), "should return error when buffer full");
        match result.unwrap_err() {
            BackpressureError::BufferFull => {} // expected
            other @ BackpressureError::NotConfigured => panic!("expected BufferFull, got: {other}"),
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
            flush_interval_ms: 60_000, // Long interval — batch_size should trigger flush
        };
        let coll_clone = coll.clone();
        let ingester = StreamIngester::new(coll, config);

        // Send exactly batch_size points
        for i in 0..batch_size {
            ingester
                .try_send(make_point(i as u64 + 1, 4))
                .expect("send should succeed");
        }

        // Give the drain loop time to process
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Verify points were upserted
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
            batch_size: 100, // Very large batch — timer should trigger flush
            flush_interval_ms: 50,
        };
        let coll_clone = coll.clone();
        let ingester = StreamIngester::new(coll, config);

        // Send only 2 points (well below batch_size of 100)
        ingester.try_send(make_point(1, 4)).expect("send 1");
        ingester.try_send(make_point(2, 4)).expect("send 2");

        // Wait longer than flush_interval_ms
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Verify points were flushed by the timer
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
            batch_size: 1000, // Very large batch — won't trigger batch flush
            flush_interval_ms: 60_000, // Very long — won't trigger timer flush
        };
        let coll_clone = coll.clone();
        let ingester = StreamIngester::new(coll, config);

        // Send a few points
        ingester.try_send(make_point(10, 4)).expect("send");
        ingester.try_send(make_point(11, 4)).expect("send");

        // Small yield to ensure points are in the channel
        tokio::task::yield_now().await;

        // Shutdown should flush remaining
        ingester.shutdown().await;

        // Verify points were flushed during shutdown
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

        // Activate delta buffer (simulating HNSW rebuild)
        coll.delta_buffer.activate();

        let ingester = StreamIngester::new(coll, config);

        // Send points via streaming
        for i in 1..=4 {
            ingester
                .try_send(make_point(i, 4))
                .expect("send should succeed");
        }

        // Wait for drain loop to flush
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Verify points are in storage (upsert always writes)
        let results = coll_clone.get(&[1, 2, 3, 4]);
        let found = results.iter().filter(|r| r.is_some()).count();
        assert_eq!(found, 4, "upsert should write all points to storage");

        // Verify points are also in the delta buffer
        assert_eq!(
            coll_clone.delta_buffer.len(),
            4,
            "delta buffer should contain the streamed points when active"
        );

        ingester.shutdown().await;
    }
}
