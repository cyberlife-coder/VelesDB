//! Lightweight counters for first-hour troubleshooting diagnostics.
//!
//! Extracted from `lib.rs` to keep module size under 500 NLOC.

use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight counters for first-hour troubleshooting diagnostics.
#[derive(Default)]
pub struct OnboardingMetrics {
    /// Total search requests received.
    pub search_requests_total: AtomicU64,
    /// Total graph requests received.
    pub graph_requests_total: AtomicU64,
    /// Total dimension mismatch errors.
    pub dimension_mismatch_total: AtomicU64,
    /// Total searches returning empty results.
    pub empty_search_results_total: AtomicU64,
    /// Total filter parse errors.
    pub filter_parse_errors_total: AtomicU64,
}

impl OnboardingMetrics {
    /// Records a search request.
    pub fn record_search_request(&self) {
        self.search_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a dimension mismatch error.
    pub fn record_dimension_mismatch(&self) {
        self.dimension_mismatch_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records an empty search result.
    pub fn record_empty_search_results(&self) {
        self.empty_search_results_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a filter parse error.
    pub fn record_filter_parse_error(&self) {
        self.filter_parse_errors_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Records a graph request.
    pub fn record_graph_request(&self) {
        self.graph_requests_total.fetch_add(1, Ordering::Relaxed);
    }
}
