//! Python binding for the streaming-ingestion configuration (STREAM-1).
//!
//! Exposes [`velesdb_core::StreamingConfig`] — the runtime configuration for
//! the bounded-channel micro-batch ingestion pipeline — as a Python dataclass.
//! This is distinct from [`crate::StreamingConfig`], which configures graph BFS
//! traversal; this type controls vector write streaming.

use pyo3::prelude::*;

/// Configuration for the streaming vector-ingestion pipeline.
///
/// Controls the bounded channel capacity, micro-batch sizing, and flush
/// timing. Unspecified fields fall back to the engine defaults
/// (`buffer_size=10000`, `batch_size=128`, `flush_interval_ms=50`).
///
/// Example:
///     >>> from velesdb import StreamingIngestConfig
///     >>> cfg = StreamingIngestConfig(buffer_size=4096, batch_size=256)
///     >>> cfg.flush_interval_ms
///     50
#[pyclass(module = "velesdb", from_py_object)]
#[derive(Clone, Debug)]
pub struct StreamingIngestConfig {
    /// Capacity of the bounded ingestion channel (backpressure threshold).
    #[pyo3(get, set)]
    pub buffer_size: usize,
    /// Number of points that trigger an immediate micro-batch flush.
    #[pyo3(get, set)]
    pub batch_size: usize,
    /// Maximum time (ms) before a partial batch is flushed.
    #[pyo3(get, set)]
    pub flush_interval_ms: u64,
}

#[pymethods]
impl StreamingIngestConfig {
    /// Creates a new config, defaulting any omitted field to the engine value.
    #[new]
    #[pyo3(signature = (buffer_size=10_000, batch_size=128, flush_interval_ms=50))]
    fn new(buffer_size: usize, batch_size: usize, flush_interval_ms: u64) -> Self {
        Self {
            buffer_size,
            batch_size,
            flush_interval_ms,
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "StreamingIngestConfig(buffer_size={}, batch_size={}, flush_interval_ms={})",
            self.buffer_size, self.batch_size, self.flush_interval_ms
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_config_defaults() {
        // The Python constructor defaults must match the core engine defaults.
        let cfg = StreamingIngestConfig::new(10_000, 128, 50);
        assert_eq!(cfg.buffer_size, 10_000);
        assert_eq!(cfg.batch_size, 128);
        assert_eq!(cfg.flush_interval_ms, 50);
    }

    #[test]
    fn test_streaming_config_overrides() {
        let cfg = StreamingIngestConfig::new(4096, 256, 10);
        assert_eq!(cfg.buffer_size, 4096);
        assert_eq!(cfg.batch_size, 256);
        assert_eq!(cfg.flush_interval_ms, 10);
        assert!(cfg.__repr__().contains("buffer_size=4096"));
    }
}
