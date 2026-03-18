//! StreamIngester: bounded-channel ingestion with micro-batch drain.

use crate::collection::types::Collection;
use crate::point::Point;

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
///
/// Future: persist StreamingConfig in CollectionConfig (STREAM-04)
///
/// `StreamingConfig` is currently runtime-only. A future pass should
/// serialize it into `CollectionConfig` so the pipeline is automatically
/// restored on `Collection::open`.
#[derive(Debug, Clone)]
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
///
/// Future: integrate WriteMode into StreamingConfig (STREAM-03)
///
/// `WriteMode` is currently unused. Once streaming-specific write paths
/// (e.g., bypass WAL for low-latency inserts) are implemented, wire this
/// into the flush pipeline.
#[allow(dead_code)] // Tracked: STREAM-03
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteMode {
    /// Standard synchronous API upsert.
    Api,
    /// Streaming micro-batch drain.
    Streaming,
}

/// Error returned when the streaming channel cannot accept a point.
#[derive(Debug, thiserror::Error)]
pub enum BackpressureError {
    /// The ingestion buffer is full; the caller should retry after a short delay.
    #[error("streaming buffer is full (backpressure)")]
    BufferFull,

    /// Streaming is not configured on this collection.
    #[error("streaming is not configured on this collection")]
    NotConfigured,

    /// The drain task has exited; the streaming pipeline is no longer functional.
    ///
    /// This is a fatal condition — the server should respond 503 Service Unavailable
    /// and the collection may need to be reconfigured.
    #[error("streaming drain task has exited; the ingestion pipeline is dead")]
    DrainTaskDead,
}

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
    /// [`BackpressureError::BufferFull`]. If the drain task has exited
    /// (channel closed), returns [`BackpressureError::DrainTaskDead`].
    ///
    /// # Errors
    ///
    /// - [`BackpressureError::BufferFull`] — the bounded channel is at capacity.
    /// - [`BackpressureError::DrainTaskDead`] — the drain task exited unexpectedly.
    pub fn try_send(&self, point: Point) -> Result<(), BackpressureError> {
        self.sender.try_send(point).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => BackpressureError::BufferFull,
            mpsc::error::TrySendError::Closed(_) => BackpressureError::DrainTaskDead,
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
            // Branch 1: shutdown signal — drain remaining channel items in
            // micro-batches (M-1: flush at batch_size to bound memory usage).
            () = shutdown.notified() => {
                while let Ok(point) = rx.try_recv() {
                    batch.push(point);
                    // Flush at batch_size boundaries to avoid unbounded accumulation.
                    if batch.len() >= batch_size {
                        flush_batch(&collection, &mut batch).await;
                    }
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

// All ingester tests live in ingester_tests.rs to keep this file under 500 NLOC.
// The sibling test file `ingester_tests.rs` is registered in `streaming/mod.rs`.
