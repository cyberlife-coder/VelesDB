//! Search quality metrics, operational monitoring, and query diagnostics.
//!
//! This module provides:
//! - **Retrieval quality**: Recall@k, Precision@k, MRR, NDCG, Hit Rate, MAP
//! - **Latency statistics**: Percentile computation (p50, p95, p99)
//! - **Operational metrics**: Prometheus-exportable counters/gauges
//! - **Query diagnostics**: Slow query logging, tracing spans, histograms
//!
//! # Example
//!
//! ```rust
//! use velesdb_core::metrics::{recall_at_k, precision_at_k, mrr};
//!
//! let ground_truth = vec![1, 2, 3, 4, 5];  // True top-5 neighbors
//! let results = vec![1, 3, 6, 2, 7];       // Retrieved results
//!
//! let recall = recall_at_k(&ground_truth, &results);      // 3/5 = 0.6
//! let precision = precision_at_k(&ground_truth, &results); // 3/5 = 0.6
//! let rank_quality = mrr(&ground_truth, &results);         // 1/1 = 1.0 (first result is relevant)
//! ```

mod latency;
mod operational;
mod query;
mod retrieval;

// Re-export retrieval quality metrics
pub use retrieval::{
    average_metrics, hit_rate, mean_average_precision, mrr, ndcg_at_k, precision_at_k, recall_at_k,
};

// Re-export latency statistics
pub use latency::{compute_latency_percentiles, LatencyStats};

// Re-export operational metrics
pub use operational::{
    GuardRailsMetrics, LimitType, OperationalMetrics, TraversalMetrics, DEPTH_BUCKETS,
    DURATION_BUCKETS, NODES_BUCKETS,
};

// Re-export query diagnostics
pub use query::{DurationHistogram, QueryPhase, QueryStats, SlowQueryLogger, SpanBuilder};
