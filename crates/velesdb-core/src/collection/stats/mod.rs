//! Collection statistics module for query planning.
//!
//! This module provides statistics collection and caching for collections,
//! enabling cost-based query planning and optimization.
//!
//! # EPIC-046 US-001: Collection Statistics
//!
//! Implements collection-level statistics including:
//! - Row count and deleted count
//! - Column cardinality (distinct values)
//! - Index statistics (depth, entry count)
//! - Size metrics (avg row size, total size)

// SAFETY: Numeric casts in statistics are intentional:
// - All casts are for computing collection metrics and estimates
// - f64/usize conversions for cardinality ratios and averages
// - Values bounded by collection size and column cardinality
// - Precision loss acceptable for statistics (approximate by design)
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod histogram;
pub(crate) use histogram::HistogramBuilder;
pub use histogram::{Histogram, HistogramBucket};

#[cfg(test)]
mod tests;

/// Statistics for a collection.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CollectionStats {
    /// Total number of points in the collection.
    pub total_points: u64,
    /// Total payload storage footprint in bytes.
    pub payload_size_bytes: u64,
    /// Per-field statistics for cost-based planning.
    pub field_stats: HashMap<String, ColumnStats>,
    /// Number of active rows
    pub row_count: u64,
    /// Number of deleted/tombstoned rows
    pub deleted_count: u64,
    /// Average row size in bytes
    pub avg_row_size_bytes: u64,
    /// Total collection size in bytes
    pub total_size_bytes: u64,
    /// Statistics per column
    pub column_stats: HashMap<String, ColumnStats>,
    /// Statistics per index
    pub index_stats: HashMap<String, IndexStats>,
    /// Timestamp of last ANALYZE
    pub last_analyzed_epoch_ms: Option<u64>,
}

impl CollectionStats {
    /// Creates empty statistics
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates statistics with basic counts
    #[must_use]
    pub fn with_counts(row_count: u64, deleted_count: u64) -> Self {
        Self {
            total_points: row_count,
            row_count,
            deleted_count,
            ..Default::default()
        }
    }

    /// Returns the live row count (excluding deleted)
    #[must_use]
    pub fn live_row_count(&self) -> u64 {
        self.row_count.saturating_sub(self.deleted_count)
    }

    /// Returns the deletion ratio (0.0-1.0)
    #[must_use]
    pub fn deletion_ratio(&self) -> f64 {
        if self.row_count == 0 {
            0.0
        } else {
            self.deleted_count as f64 / self.row_count as f64
        }
    }

    /// Estimates selectivity for a column based on cardinality
    #[must_use]
    pub fn estimate_selectivity(&self, column: &str) -> f64 {
        if let Some(col_stats) = self.field_stats.get(column) {
            if col_stats.distinct_values > 0 && self.total_points > 0 {
                return 1.0 / col_stats.distinct_values as f64;
            }
        }
        if let Some(col_stats) = self.column_stats.get(column) {
            if col_stats.distinct_count > 0 && self.row_count > 0 {
                return 1.0 / col_stats.distinct_count as f64;
            }
        }
        // Default: assume 10% selectivity if unknown
        0.1
    }

    /// Returns the histogram for a column, checking both `column_stats` and `field_stats`.
    ///
    /// Returns `None` when neither map contains the column or the histogram is
    /// absent / empty.
    #[must_use]
    pub fn get_column_histogram(&self, column: &str) -> Option<&Histogram> {
        self.column_stats
            .get(column)
            .or_else(|| self.field_stats.get(column))
            .and_then(|cs| cs.histogram.as_ref())
            .filter(|h| !h.buckets.is_empty())
    }

    /// Sets the last analyzed timestamp to now
    pub fn mark_analyzed(&mut self) {
        self.last_analyzed_epoch_ms = Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        );
    }
}

/// Statistics for a single column.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ColumnStats {
    /// Column name
    pub name: String,
    /// Number of null values
    pub null_count: u64,
    /// Number of distinct values (cardinality)
    pub distinct_count: u64,
    /// Number of distinct values (CBO alias).
    pub distinct_values: u64,
    /// Minimum value (serialized)
    pub min_value: Option<String>,
    /// Maximum value (serialized)
    pub max_value: Option<String>,
    /// Average value size in bytes
    pub avg_size_bytes: u64,
    /// Optional histogram for selectivity estimates.
    pub histogram: Option<Histogram>,
}

impl ColumnStats {
    /// Creates new column stats
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Sets cardinality
    #[must_use]
    pub fn with_distinct_count(mut self, count: u64) -> Self {
        self.distinct_count = count;
        self.distinct_values = count;
        self
    }

    /// Sets null count
    #[must_use]
    pub fn with_null_count(mut self, count: u64) -> Self {
        self.null_count = count;
        self
    }
}

/// Statistics for an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    /// Index name
    pub name: String,
    /// Index type (HNSW, PropertyIndex, etc.)
    pub index_type: String,
    /// Number of entries in the index
    pub entry_count: u64,
    /// Index depth (for tree-based indexes)
    pub depth: u32,
    /// Index size in bytes
    pub size_bytes: u64,
}

impl IndexStats {
    /// Creates new index stats
    #[must_use]
    pub fn new(name: impl Into<String>, index_type: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            index_type: index_type.into(),
            ..Default::default()
        }
    }

    /// Sets entry count
    #[must_use]
    pub fn with_entry_count(mut self, count: u64) -> Self {
        self.entry_count = count;
        self
    }

    /// Sets depth
    #[must_use]
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }
}

/// Statistics collector for building CollectionStats.
#[derive(Debug, Default)]
pub struct StatsCollector {
    stats: CollectionStats,
}

impl StatsCollector {
    /// Creates a new collector
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets row count
    pub fn set_row_count(&mut self, count: u64) {
        self.stats.row_count = count;
        self.stats.total_points = count;
    }

    /// Sets deleted count
    pub fn set_deleted_count(&mut self, count: u64) {
        self.stats.deleted_count = count;
    }

    /// Sets total size
    pub fn set_total_size(&mut self, size: u64) {
        self.stats.total_size_bytes = size;
        self.stats.payload_size_bytes = size;
    }

    /// Adds column statistics
    pub fn add_column_stats(&mut self, stats: ColumnStats) {
        self.stats
            .column_stats
            .insert(stats.name.clone(), stats.clone());
        self.stats.field_stats.insert(stats.name.clone(), stats);
    }

    /// Adds index statistics
    pub fn add_index_stats(&mut self, stats: IndexStats) {
        self.stats.index_stats.insert(stats.name.clone(), stats);
    }

    /// Builds a histogram for a column from sampled values and stores it.
    ///
    /// Called by `Collection::analyze()` for each Int, Float, and String column.
    /// Uses `HistogramBuilder` to construct an equi-depth histogram, then attaches
    /// it to the corresponding `ColumnStats` entry (creating one if absent).
    pub fn build_histogram(&mut self, column_name: &str, values: &mut [f64], num_buckets: usize) {
        let histogram = HistogramBuilder::new(num_buckets).build(values);
        if let Some(col) = self.stats.column_stats.get_mut(column_name) {
            col.histogram = Some(histogram.clone());
        }
        if let Some(col) = self.stats.field_stats.get_mut(column_name) {
            col.histogram = Some(histogram);
        }
    }

    /// Builds the final CollectionStats
    #[must_use]
    pub fn build(mut self) -> CollectionStats {
        // Calculate average row size
        if self.stats.row_count > 0 {
            self.stats.avg_row_size_bytes = self.stats.total_size_bytes / self.stats.row_count;
        }

        self.stats.mark_analyzed();
        self.stats
    }
}
