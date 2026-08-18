//! CBO calibration feedback loop (issue #469).
//!
//! Adjusts `ms_per_cost_unit` toward observed query latencies via an
//! exponential moving average (EMA) with α=0.05.  Conservative rate prevents
//! over-fitting to short query bursts.
//!
//! # Algorithm
//!
//! After each vector search, compute:
//! ```text
//! estimated_cost  = log2(n + 1) × (ef / 100)   // same O(log n) model as QueryCostEstimator
//! observed_ratio  = actual_ms / estimated_cost
//! ema             = α × observed_ratio + (1 − α) × ema
//! ```
//!
//! Outlier rejection: if `observed_ratio / ema > 10`, the sample is noise
//! (cold cache, GC pause, OS jitter) and is discarded.
//!
//! Adjustment activates only after [`MIN_SAMPLES`] observations, giving the
//! EMA time to warm up before influencing planner decisions.
//!
//! # Thread safety
//!
//! All state is held in `AtomicU64` fields.  The EMA is updated with a
//! compare-and-swap loop identical to the one in
//! [`crate::velesql::query_stats`].

use std::sync::atomic::{AtomicU64, Ordering};

/// Minimum observations before the EMA value influences the planner.
const MIN_SAMPLES: u64 = 10;

/// EMA learning rate α (5 %).  Conservative to avoid over-fitting.
const ALPHA_NUMERATOR: u64 = 5;
const ALPHA_DENOMINATOR: u64 = 100;

/// Outlier rejection threshold: skip if observed ÷ EMA > 10×.
const OUTLIER_RATIO: f64 = 10.0;

/// Safety bounds for the adjusted ms_per_cost_unit.
const MIN_MS_PER_UNIT: f64 = 0.001;
const MAX_MS_PER_UNIT: f64 = 50.0;

/// Scale factor for storing f64 in AtomicU64 (×1 000 000 → sub-microsecond precision).
const SCALE: f64 = 1_000_000.0;

/// Lock-free EMA-based feedback loop for CBO cost-unit calibration.
///
/// Crate-internal: instantiated by [`crate::velesql::planner::QueryPlanner`] and
/// fed by the query pipeline. Not part of the public crate API.
#[derive(Debug, Default)]
pub(crate) struct CboFeedbackLoop {
    /// EMA of observed ms-per-cost-unit (stored as u64 = value × SCALE).
    ema_scaled: AtomicU64,
    /// Total samples recorded.
    sample_count: AtomicU64,
}

impl CboFeedbackLoop {
    /// Creates a new, empty feedback loop.
    #[must_use]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records an observation and updates the EMA.
    ///
    /// `dataset_size` — number of indexed vectors (used to estimate cost).
    /// `ef_search`    — effective ef_search used for this query.
    /// `actual_ms`    — wall-clock duration of the query in milliseconds.
    pub(crate) fn record(&self, dataset_size: usize, ef_search: usize, actual_ms: f64) {
        if actual_ms <= 0.0 || dataset_size == 0 {
            return;
        }

        let estimated_cost = Self::estimate_cost(dataset_size, ef_search);
        if estimated_cost <= 0.0 {
            return;
        }

        let observed_ratio = actual_ms / estimated_cost;

        // Reject outliers once the EMA has warmed up.
        let count = self.sample_count.load(Ordering::Relaxed);
        if count >= MIN_SAMPLES {
            let current_ema = self.current_ema();
            if current_ema > 0.0 && observed_ratio / current_ema > OUTLIER_RATIO {
                return;
            }
        }

        self.sample_count.fetch_add(1, Ordering::Relaxed);
        self.ema_update(observed_ratio);
    }

    /// Returns the calibrated `ms_per_cost_unit` after sufficient observations.
    ///
    /// Returns `None` until at least [`MIN_SAMPLES`] observations have been
    /// recorded, so the planner falls back to the static default during warm-up.
    #[must_use]
    pub(crate) fn adjusted_ms_per_cost_unit(&self) -> Option<f64> {
        if self.sample_count.load(Ordering::Relaxed) < MIN_SAMPLES {
            return None;
        }
        let v = self.current_ema();
        if v > 0.0 {
            Some(v.clamp(MIN_MS_PER_UNIT, MAX_MS_PER_UNIT))
        } else {
            None
        }
    }

    /// Returns the total number of samples recorded.
    #[must_use]
    pub(crate) fn sample_count(&self) -> u64 {
        self.sample_count.load(Ordering::Relaxed)
    }

    /// Returns the current EMA value (used internally for outlier rejection and
    /// surfaced via [`Self::adjusted_ms_per_cost_unit`]).
    #[must_use]
    fn current_ema(&self) -> f64 {
        // u64 → f64: values are bounded by MAX_MS_PER_UNIT × SCALE = 50_000_000,
        // well within f64's exact integer range (2^53 ≈ 9 × 10^15).
        #[allow(clippy::cast_precision_loss)]
        let scaled = self.ema_scaled.load(Ordering::Relaxed) as f64;
        scaled / SCALE
    }

    // -------------------------------------------------------------------------
    // Internals
    // -------------------------------------------------------------------------

    /// Simplified cost model matching `QueryCostEstimator::estimate`.
    ///
    /// Uses the O(log n) × ef_search component only (no top-k, no filter),
    /// which is the dominant term for the feedback signal.
    fn estimate_cost(dataset_size: usize, ef_search: usize) -> f64 {
        // usize → f64: realistic collection sizes fit within f64's exact integer range.
        #[allow(clippy::cast_precision_loss)]
        let n_factor = (dataset_size as f64 + 1.0).log2();
        #[allow(clippy::cast_precision_loss)]
        let ef_factor = ef_search as f64 / 100.0;
        n_factor * ef_factor
    }

    /// CAS-loop EMA update with α = `ALPHA_NUMERATOR` / `ALPHA_DENOMINATOR`.
    fn ema_update(&self, new_value: f64) {
        // Clamp to [0, MAX_MS_PER_UNIT] before scaling so the cast is always safe:
        // after clamp, max value = MAX_MS_PER_UNIT × SCALE = 50_000_000 << u64::MAX.
        let clamped = new_value.clamp(0.0, MAX_MS_PER_UNIT);
        // f64 → u64: safe — value is clamped, finite, and non-negative.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let new_scaled = (clamped * SCALE) as u64;

        loop {
            let old_scaled = self.ema_scaled.load(Ordering::Relaxed);
            let new_ema_scaled = if old_scaled == 0 {
                new_scaled
            } else {
                // EMA: result = α × new + (1−α) × old.
                // Use u128 intermediates to make overflow impossibility self-evident:
                // max product = 50_000_000 × 95 = 4.75 × 10^9 << u128::MAX.
                let num = u128::from(new_scaled) * u128::from(ALPHA_NUMERATOR)
                    + u128::from(old_scaled) * u128::from(ALPHA_DENOMINATOR - ALPHA_NUMERATOR);
                // Result ≤ max(new_scaled, old_scaled) ≤ 50_000_000 — fits in u64.
                #[allow(clippy::cast_possible_truncation)]
                let result = (num / u128::from(ALPHA_DENOMINATOR)) as u64;
                result
            };
            if self
                .ema_scaled
                .compare_exchange_weak(
                    old_scaled,
                    new_ema_scaled,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                break;
            }
        }
    }
}

#[cfg(test)]
#[path = "feedback_tests.rs"]
mod tests;
