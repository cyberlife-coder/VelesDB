//! Auto-reindex module for automatic HNSW index optimization.
//!
//! This module provides automatic detection and triggering of HNSW reindex
//! operations when index parameters become suboptimal for the current dataset size.
//!
//! # Features
//!
//! - **Parameter mismatch detection**: Compares current vs optimal params
//! - **Background reindexing**: Non-blocking index rebuild
//! - **Automatic rollback**: Reverts if new index performs worse
//! - **Event emission**: Notifies of reindex lifecycle events
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::collection::auto_reindex::{AutoReindexConfig, AutoReindexManager};
//!
//! let config = AutoReindexConfig::default();
//! let manager = AutoReindexManager::new(config);
//!
//! // Check if reindex is needed
//! if manager.should_reindex(current_params, current_size, dimension) {
//!     manager.trigger_reindex();
//! }
//! ```

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::index::hnsw::HnswParams;

mod types;

#[cfg(test)]
mod tests;

pub use types::{
    AutoReindexConfig, BenchmarkResult, DivergenceCheck, ReindexEvent, ReindexReason, ReindexState,
};

/// Type alias for reindex event callback
type EventCallback = Arc<dyn Fn(ReindexEvent) + Send + Sync>;

/// Manages automatic reindexing for a collection
pub struct AutoReindexManager {
    /// Configuration
    config: RwLock<AutoReindexConfig>,
    /// Current state
    state: AtomicU8,
    /// Event callback
    event_callback: RwLock<Option<EventCallback>>,
    /// Last reindex timestamp (for cooldown)
    last_reindex_timestamp: RwLock<Option<std::time::Instant>>,
}

impl AutoReindexManager {
    /// Creates a new manager with the given configuration
    #[must_use]
    pub fn new(config: AutoReindexConfig) -> Self {
        Self {
            config: RwLock::new(config),
            state: AtomicU8::new(ReindexState::Idle as u8),
            event_callback: RwLock::new(None),
            last_reindex_timestamp: RwLock::new(None),
        }
    }

    /// Creates a new manager with default configuration
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(AutoReindexConfig::default())
    }

    /// Returns the current state
    #[must_use]
    pub fn state(&self) -> ReindexState {
        ReindexState::from(self.state.load(Ordering::Acquire))
    }

    /// Returns whether auto-reindex is enabled
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.config.read().expect("config lock poisoned").enabled
    }

    /// Enables or disables auto-reindex
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    pub fn set_enabled(&self, enabled: bool) {
        self.config.write().expect("config lock poisoned").enabled = enabled;
    }

    /// Updates the configuration
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    pub fn set_config(&self, config: AutoReindexConfig) {
        *self.config.write().expect("config lock poisoned") = config;
    }

    /// Gets the current configuration
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    #[must_use]
    pub fn config(&self) -> AutoReindexConfig {
        self.config.read().expect("config lock poisoned").clone()
    }

    /// Sets the event callback for reindex lifecycle events
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    pub fn on_event<F>(&self, callback: F)
    where
        F: Fn(ReindexEvent) + Send + Sync + 'static,
    {
        *self.event_callback.write().expect("callback lock poisoned") = Some(Arc::new(callback));
    }

    /// Emits an event to the registered callback
    fn emit_event(&self, event: ReindexEvent) {
        if let Ok(guard) = self.event_callback.read() {
            if let Some(ref callback) = *guard {
                callback(event);
            }
        }
    }

    /// Checks if parameters have diverged enough to warrant reindex
    ///
    /// # Arguments
    ///
    /// * `current_params` - Current HNSW parameters
    /// * `current_size` - Current number of vectors
    /// * `dimension` - Vector dimension
    ///
    /// # Returns
    ///
    /// `DivergenceCheck` with recommendation and details
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    #[must_use]
    pub fn check_divergence(
        &self,
        current_params: &HnswParams,
        current_size: usize,
        dimension: usize,
    ) -> DivergenceCheck {
        let config = self.config.read().expect("config lock poisoned");

        // Check minimum size
        if current_size < config.min_size_for_reindex {
            return DivergenceCheck {
                should_reindex: false,
                current_m: current_params.max_connections,
                optimal_m: current_params.max_connections,
                ratio: 1.0,
                reason: None,
            };
        }

        // Get optimal params for current size
        let optimal_params = HnswParams::for_dataset_size(dimension, current_size);
        let current_m = current_params.max_connections;
        let optimal_m = optimal_params.max_connections;

        // Calculate ratio (how much larger optimal is compared to current)
        let ratio = if current_m > 0 {
            optimal_m as f64 / current_m as f64
        } else {
            f64::INFINITY
        };

        let should_reindex = config.enabled && ratio >= config.param_divergence_threshold;

        let reason = if should_reindex {
            Some(ReindexReason::ParamDivergence {
                current_m,
                optimal_m,
                ratio,
            })
        } else {
            None
        };

        DivergenceCheck {
            should_reindex,
            current_m,
            optimal_m,
            ratio,
            reason,
        }
    }

    /// Checks if reindex should be triggered (convenience method)
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    #[must_use]
    pub fn should_reindex(
        &self,
        current_params: &HnswParams,
        current_size: usize,
        dimension: usize,
    ) -> bool {
        // Check cooldown
        if let Ok(guard) = self.last_reindex_timestamp.read() {
            if let Some(last) = *guard {
                let config = self.config.read().expect("config lock poisoned");
                if last.elapsed() < config.cooldown {
                    return false;
                }
            }
        }

        // Check if already reindexing
        if self.state() != ReindexState::Idle {
            return false;
        }

        self.check_divergence(current_params, current_size, dimension)
            .should_reindex
    }

    /// Validates that new index performs at least as well as old
    ///
    /// Returns `Ok(())` if validation passes, `Err(reason)` if rollback needed
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    pub fn validate_benchmark(
        &self,
        old_benchmark: &BenchmarkResult,
        new_benchmark: &BenchmarkResult,
    ) -> Result<(), String> {
        let config = self.config.read().expect("config lock poisoned");

        // Check latency regression
        if old_benchmark.latency_p99_us > 0 {
            let latency_change = (new_benchmark.latency_p99_us as f64
                - old_benchmark.latency_p99_us as f64)
                / old_benchmark.latency_p99_us as f64
                * 100.0;

            if latency_change > config.max_latency_regression_percent {
                return Err(format!(
                    "Latency regression: {:.1}% (max allowed: {:.1}%)",
                    latency_change, config.max_latency_regression_percent
                ));
            }
        }

        // Check recall regression
        if old_benchmark.recall_estimate > 0.0 {
            let recall_change =
                (old_benchmark.recall_estimate - new_benchmark.recall_estimate) * 100.0;

            if recall_change > config.max_recall_regression_percent {
                return Err(format!(
                    "Recall regression: {:.1}% (max allowed: {:.1}%)",
                    recall_change, config.max_recall_regression_percent
                ));
            }
        }

        Ok(())
    }

    /// Transitions to a new state
    fn transition_to(&self, new_state: ReindexState) -> bool {
        let current = self.state.load(Ordering::Acquire);
        self.state
            .compare_exchange(
                current,
                new_state as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    /// Starts the reindex process (for manual trigger)
    ///
    /// Returns `true` if reindex was started, `false` if already in progress
    pub fn trigger_manual_reindex(&self) -> bool {
        if self.state() != ReindexState::Idle {
            return false;
        }

        if self.transition_to(ReindexState::Building) {
            self.emit_event(ReindexEvent::Started {
                reason: ReindexReason::Manual,
                old_params: HnswParams::default(),
                new_params: HnswParams::default(),
            });
            true
        } else {
            false
        }
    }

    /// Starts the reindex process with specific parameters
    pub fn start_reindex(
        &self,
        reason: ReindexReason,
        old_params: HnswParams,
        new_params: HnswParams,
    ) -> bool {
        if self.state() != ReindexState::Idle {
            return false;
        }

        if self.transition_to(ReindexState::Building) {
            self.emit_event(ReindexEvent::Started {
                reason,
                old_params,
                new_params,
            });
            true
        } else {
            false
        }
    }

    /// Updates progress (0-100)
    pub fn report_progress(&self, percent: u8) {
        if self.state() == ReindexState::Building {
            self.emit_event(ReindexEvent::Progress {
                percent: percent.min(100),
            });
        }
    }

    /// Transitions to validation phase
    pub fn start_validation(&self, old_latency_p99_us: u64, new_latency_p99_us: u64) -> bool {
        if self.state() != ReindexState::Building {
            return false;
        }

        if self.transition_to(ReindexState::Validating) {
            self.emit_event(ReindexEvent::Validating {
                old_latency_p99_us,
                new_latency_p99_us,
            });
            true
        } else {
            false
        }
    }

    /// Completes the reindex successfully
    ///
    /// # Panics
    ///
    /// Panics if the internal RwLock is poisoned.
    pub fn complete_reindex(&self, duration: Duration) -> bool {
        if self.state() != ReindexState::Validating && self.state() != ReindexState::Swapping {
            return false;
        }

        // Update last reindex timestamp
        *self
            .last_reindex_timestamp
            .write()
            .expect("timestamp lock poisoned") = Some(std::time::Instant::now());

        self.state
            .store(ReindexState::Idle as u8, Ordering::Release);
        self.emit_event(ReindexEvent::Completed { duration });
        true
    }

    /// Rolls back the reindex due to regression or error
    pub fn rollback(&self, reason: String) -> bool {
        let current_state = self.state();
        if current_state == ReindexState::Idle {
            return false;
        }

        self.state
            .store(ReindexState::Idle as u8, Ordering::Release);
        self.emit_event(ReindexEvent::RolledBack { reason });
        true
    }

    /// Resets to idle state (for testing or error recovery)
    pub fn reset(&self) {
        self.state
            .store(ReindexState::Idle as u8, Ordering::Release);
    }
}

impl Default for AutoReindexManager {
    fn default() -> Self {
        Self::with_defaults()
    }
}
