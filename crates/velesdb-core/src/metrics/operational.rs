//! Operational metrics for monitoring `VelesDB` in production.
//!
//! Provides thread-safe counters and gauges for:
//! - Query throughput and errors (Prometheus-exportable)
//! - Graph traversal statistics
//! - Guard-rails and rate limiting metrics

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Query duration histogram buckets (in seconds).
pub const DURATION_BUCKETS: [f64; 8] = [0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0];

/// Traversal depth histogram buckets.
pub const DEPTH_BUCKETS: [u64; 6] = [1, 2, 3, 5, 10, 20];

/// Nodes visited histogram buckets.
pub const NODES_BUCKETS: [u64; 7] = [10, 50, 100, 500, 1000, 5000, 10000];

/// Operational metrics for `VelesDB` monitoring (EPIC-050 US-001).
///
/// Thread-safe counters and gauges that can be exported in Prometheus format.
#[derive(Debug, Default)]
pub struct OperationalMetrics {
    /// Total queries executed
    pub queries_total: AtomicU64,
    /// Total query errors
    pub query_errors: AtomicU64,
    /// Vector search queries
    pub vector_queries: AtomicU64,
    /// Graph traversal queries
    pub graph_queries: AtomicU64,
    /// Hybrid queries (vector + graph)
    pub hybrid_queries: AtomicU64,
    /// Total documents across all collections
    pub documents_total: AtomicU64,
    /// Total index size in bytes
    pub index_size_bytes: AtomicU64,
    /// Active connections (for server)
    pub active_connections: AtomicU64,
}

impl OperationalMetrics {
    /// Creates a new metrics instance.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a shared metrics instance.
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Increments the total query counter.
    pub fn inc_queries(&self) {
        self.queries_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Increments the query error counter.
    pub fn inc_errors(&self) {
        self.query_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a vector search query.
    pub fn record_vector_query(&self) {
        self.inc_queries();
        self.vector_queries.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a graph traversal query.
    pub fn record_graph_query(&self) {
        self.inc_queries();
        self.graph_queries.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a hybrid query.
    pub fn record_hybrid_query(&self) {
        self.inc_queries();
        self.hybrid_queries.fetch_add(1, Ordering::Relaxed);
    }

    /// Sets the document count.
    pub fn set_documents(&self, count: u64) {
        self.documents_total.store(count, Ordering::Relaxed);
    }

    /// Sets the index size.
    pub fn set_index_size(&self, bytes: u64) {
        self.index_size_bytes.store(bytes, Ordering::Relaxed);
    }

    /// Increments active connections.
    pub fn inc_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrements active connections.
    ///
    /// Uses a CAS loop to saturate at 0, preventing underflow wrap to `u64::MAX`.
    pub fn dec_connections(&self) {
        // BUG-3 FIX: Use CAS loop to prevent underflow
        loop {
            let current = self.active_connections.load(Ordering::Relaxed);
            if current == 0 {
                return; // Already at 0, don't underflow
            }
            if self
                .active_connections
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
            // CAS failed, retry
        }
    }

    /// Exports metrics in Prometheus text format.
    #[must_use]
    pub fn export_prometheus(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();

        // Queries total
        // BUG-4 FIX: success = total - errors, not total (which includes errors)
        let total = self.queries_total.load(Ordering::Relaxed);
        let errors = self.query_errors.load(Ordering::Relaxed);
        let success = total.saturating_sub(errors);

        output.push_str("# HELP velesdb_queries_total Total number of queries executed\n");
        output.push_str("# TYPE velesdb_queries_total counter\n");
        let _ = writeln!(
            output,
            "velesdb_queries_total{{status=\"success\"}} {success}"
        );
        let _ = writeln!(
            output,
            "velesdb_queries_total{{status=\"error\"}} {errors}\n"
        );

        // Query types
        output.push_str("# HELP velesdb_queries_by_type Queries by type\n");
        output.push_str("# TYPE velesdb_queries_by_type counter\n");
        let _ = writeln!(
            output,
            "velesdb_queries_by_type{{type=\"vector\"}} {}",
            self.vector_queries.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "velesdb_queries_by_type{{type=\"graph\"}} {}",
            self.graph_queries.load(Ordering::Relaxed)
        );
        let _ = writeln!(
            output,
            "velesdb_queries_by_type{{type=\"hybrid\"}} {}\n",
            self.hybrid_queries.load(Ordering::Relaxed)
        );

        // Documents
        output.push_str("# HELP velesdb_documents_total Total documents in database\n");
        output.push_str("# TYPE velesdb_documents_total gauge\n");
        let _ = writeln!(
            output,
            "velesdb_documents_total {}\n",
            self.documents_total.load(Ordering::Relaxed)
        );

        // Index size
        output.push_str("# HELP velesdb_index_size_bytes Total index size in bytes\n");
        output.push_str("# TYPE velesdb_index_size_bytes gauge\n");
        let _ = writeln!(
            output,
            "velesdb_index_size_bytes {}\n",
            self.index_size_bytes.load(Ordering::Relaxed)
        );

        // Active connections
        output.push_str("# HELP velesdb_active_connections Current active connections\n");
        output.push_str("# TYPE velesdb_active_connections gauge\n");
        let _ = writeln!(
            output,
            "velesdb_active_connections {}",
            self.active_connections.load(Ordering::Relaxed)
        );

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operational_metrics_counters() {
        let metrics = OperationalMetrics::new();

        metrics.record_vector_query();
        metrics.record_vector_query();
        metrics.record_graph_query();
        metrics.record_hybrid_query();
        metrics.inc_errors();

        assert_eq!(metrics.queries_total.load(Ordering::Relaxed), 4);
        assert_eq!(metrics.vector_queries.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.graph_queries.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.hybrid_queries.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.query_errors.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_operational_metrics_gauges() {
        let metrics = OperationalMetrics::new();

        metrics.set_documents(1000);
        metrics.set_index_size(1024 * 1024);
        metrics.inc_connections();
        metrics.inc_connections();
        metrics.dec_connections();

        assert_eq!(metrics.documents_total.load(Ordering::Relaxed), 1000);
        assert_eq!(
            metrics.index_size_bytes.load(Ordering::Relaxed),
            1024 * 1024
        );
        assert_eq!(metrics.active_connections.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_operational_metrics_prometheus_export() {
        let metrics = OperationalMetrics::new();
        metrics.record_vector_query();
        metrics.set_documents(100);

        let output = metrics.export_prometheus();

        assert!(output.contains("velesdb_queries_total"));
        assert!(output.contains("velesdb_documents_total 100"));
        assert!(output.contains("# TYPE"));
        assert!(output.contains("# HELP"));
    }

    #[test]
    fn test_operational_metrics_shared() {
        let metrics = OperationalMetrics::shared();
        metrics.record_vector_query();

        // Clone Arc and verify shared state
        let metrics2 = Arc::clone(&metrics);
        metrics2.record_vector_query();

        assert_eq!(metrics.queries_total.load(Ordering::Relaxed), 2);
    }
}
