//! WAL group commit batcher for amortized fsync cost.
//!
//! Accumulates write operations and flushes them in a single `sync_all()` call,
//! reducing I/O overhead for concurrent workloads.
//!
//! # Design
//!
//! The batcher sits in front of a WAL writer and accumulates serialized entries
//! in an in-memory buffer. A flush is triggered when either:
//!
//! - The batch reaches `max_batch_size` entries (count-based trigger), or
//! - The caller explicitly invokes [`WalBatcher::flush`].
//!
//! When batching is disabled (`enabled = false`), [`WalBatcher::submit`] writes
//! and flushes each entry immediately as a pass-through.

use crate::config::WalBatchConfig;
use parking_lot::Mutex;
use std::io::{self, Write};

/// A batch of pending WAL entries awaiting flush.
struct PendingBatch {
    /// Accumulated serialized bytes to write.
    buffer: Vec<u8>,
    /// Number of entries in the current batch.
    entry_count: usize,
}

/// Groups multiple WAL writes into a single fsync for throughput.
///
/// Thread-safe: multiple writers can call [`submit`](Self::submit) concurrently.
/// The writer whose submission triggers the batch-full condition performs the
/// actual I/O; all other writers return immediately after appending.
///
/// # Examples
///
/// ```rust,no_run
/// use velesdb_core::config::WalBatchConfig;
/// use velesdb_core::storage::wal_batcher::WalBatcher;
///
/// let batcher = WalBatcher::new(WalBatchConfig {
///     enabled: true,
///     commit_delay_us: 100,
///     max_batch_size: 4,
///     ..Default::default()
/// });
///
/// let mut writer = Vec::new(); // any impl Write
/// batcher.submit(b"entry-1", &mut writer).unwrap();
/// batcher.submit(b"entry-2", &mut writer).unwrap();
/// // Explicit flush drains remaining entries
/// batcher.flush(&mut writer).unwrap();
/// ```
pub struct WalBatcher {
    /// User-provided configuration.
    config: WalBatchConfig,
    /// Lock-protected pending batch state.
    pending: Mutex<PendingBatch>,
}

impl WalBatcher {
    /// Creates a new batcher with the given configuration.
    #[must_use]
    pub fn new(config: WalBatchConfig) -> Self {
        Self {
            pending: Mutex::new(PendingBatch {
                buffer: Vec::new(),
                entry_count: 0,
            }),
            config,
        }
    }

    /// Submits an entry to the batch.
    ///
    /// When batching is disabled, the entry is written and flushed immediately.
    /// When batching is enabled, the entry is appended to the internal buffer
    /// and a flush is triggered only when `max_batch_size` is reached.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the underlying write or flush fails.
    pub fn submit(&self, data: &[u8], writer: &mut impl Write) -> io::Result<()> {
        if !self.config.enabled {
            return write_and_flush(writer, data);
        }

        let needs_flush = {
            let mut batch = self.pending.lock();
            batch.buffer.extend_from_slice(data);
            batch.entry_count += 1;
            batch.entry_count >= self.config.max_batch_size
        };

        if needs_flush {
            self.flush(writer)?;
        }

        Ok(())
    }

    /// Forces a flush of all pending entries to the writer.
    ///
    /// If the buffer is empty this is a no-op.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the underlying write or flush fails.
    pub fn flush(&self, writer: &mut impl Write) -> io::Result<()> {
        let data = {
            let mut batch = self.pending.lock();
            if batch.entry_count == 0 {
                return Ok(());
            }
            let data = std::mem::take(&mut batch.buffer);
            batch.entry_count = 0;
            data
        };

        writer.write_all(&data)?;
        writer.flush()
    }

    /// Returns `true` if batching is enabled.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Returns the number of entries currently pending in the batch.
    ///
    /// Useful for diagnostics and testing.
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.pending.lock().entry_count
    }

    /// Returns the byte size of the currently buffered data.
    ///
    /// Useful for diagnostics and monitoring memory pressure.
    #[must_use]
    pub fn pending_bytes(&self) -> usize {
        self.pending.lock().buffer.len()
    }
}

/// Writes data and flushes the writer in a single operation.
fn write_and_flush(writer: &mut impl Write, data: &[u8]) -> io::Result<()> {
    writer.write_all(data)?;
    writer.flush()
}
