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

// Reason: u64 bit-casting and f64/u64 conversions are intentional for
// lock-free atomic EMA. Values are bounded by MAX_MS_PER_UNIT and safe.
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]

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
#[derive(Debug, Default)]
pub struct CboFeedbackLoop {
    /// EMA of observed ms-per-cost-unit (stored as u64 = value × SCALE).
    ema_scaled: AtomicU64,
    /// Total samples recorded.
    sample_count: AtomicU64,
}

impl CboFeedbackLoop {
    /// Creates a new, empty feedback loop.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an observation and updates the EMA.
    ///
    /// `dataset_size` — number of indexed vectors (used to estimate cost).
    /// `ef_search`    — effective ef_search used for this query.
    /// `actual_ms`    — wall-clock duration of the query in milliseconds.
    pub fn record(&self, dataset_size: usize, ef_search: usize, actual_ms: f64) {
        if actual_ms <= 0.0 || dataset_size == 0 {
            return;
        }

        let estimated_cost = self.estimate_cost(dataset_size, ef_search);
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
    pub fn adjusted_ms_per_cost_unit(&self) -> Option<f64> {
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
    pub fn sample_count(&self) -> u64 {
        self.sample_count.load(Ordering::Relaxed)
    }

    /// Returns the current EMA value (for monitoring/EXPLAIN).
    #[must_use]
    pub fn current_ema(&self) -> f64 {
        self.ema_scaled.load(Ordering::Relaxed) as f64 / SCALE
    }

    // -------------------------------------------------------------------------
    // Internals
    // -------------------------------------------------------------------------

    /// Simplified cost model matching `QueryCostEstimator::estimate`.
    ///
    /// Uses the O(log n) × ef_search component only (no top-k, no filter),
    /// which is the dominant term for the feedback signal.
    fn estimate_cost(&self, dataset_size: usize, ef_search: usize) -> f64 {
        let n_factor = (dataset_size as f64 + 1.0).log2();
        let ef_factor = ef_search as f64 / 100.0;
        n_factor * ef_factor
    }

    /// CAS-loop EMA update with α=ALPHA_NUMERATOR/ALPHA_DENOMINATOR.
    fn ema_update(&self, new_value: f64) {
        let new_scaled = (new_value * SCALE) as u64;
        loop {
            let old_scaled = self.ema_scaled.load(Ordering::Relaxed);
            let new_ema_scaled = if old_scaled == 0 {
                new_scaled
            } else {
                // EMA: new = α × new + (1-α) × old
                (new_scaled * ALPHA_NUMERATOR + old_scaled * (ALPHA_DENOMINATOR - ALPHA_NUMERATOR))
                    / ALPHA_DENOMINATOR
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
mod tests {
    use super::*;

    #[test]
    fn test_no_adjustment_before_min_samples() {
        let fb = CboFeedbackLoop::new();
        for _ in 0..(MIN_SAMPLES - 1) {
            fb.record(10_000, 100, 5.0);
        }
        assert!(
            fb.adjusted_ms_per_cost_unit().is_none(),
            "should return None until MIN_SAMPLES observations"
        );
    }

    #[test]
    fn test_adjustment_after_min_samples() {
        let fb = CboFeedbackLoop::new();
        for _ in 0..MIN_SAMPLES {
            fb.record(10_000, 100, 5.0);
        }
        let adjusted = fb.adjusted_ms_per_cost_unit();
        assert!(adjusted.is_some(), "should return Some after MIN_SAMPLES");
        let v = adjusted.unwrap();
        assert!(
            v >= MIN_MS_PER_UNIT && v <= MAX_MS_PER_UNIT,
            "adjusted value {v} out of bounds"
        );
    }

    #[test]
    fn test_ema_converges_toward_observed_ratio() {
        let fb = CboFeedbackLoop::new();
        // 10K vectors, ef=100 → estimated_cost ≈ log2(10001) * 1.0 ≈ 13.29
        // actual_ms = 2.0 → target ratio ≈ 0.15
        for _ in 0..50 {
            fb.record(10_000, 100, 2.0);
        }
        let v = fb.adjusted_ms_per_cost_unit().expect("should have value");
        // After 50 iterations, EMA should be close to target ratio 0.15
        // (within ±0.05 given α=0.05 convergence speed)
        let expected = 2.0 / (10_001_f64.log2() * 1.0);
        assert!(
            (v - expected).abs() < 0.05,
            "EMA {v:.4} should be near expected {expected:.4}"
        );
    }

    #[test]
    fn test_outlier_rejection() {
        let fb = CboFeedbackLoop::new();
        // Warm up with stable observations
        for _ in 0..20 {
            fb.record(10_000, 100, 2.0);
        }
        let before = fb.current_ema();
        let before_count = fb.sample_count();

        // Inject a massive outlier (10 000× the normal value)
        fb.record(10_000, 100, 20_000.0);

        let after = fb.current_ema();
        let after_count = fb.sample_count();

        assert_eq!(
            before_count, after_count,
            "outlier should be rejected, sample count unchanged"
        );
        assert!(
            (after - before).abs() < f64::EPSILON,
            "EMA should be unchanged after outlier rejection"
        );
    }

    #[test]
    fn test_zero_or_negative_actual_ms_ignored() {
        let fb = CboFeedbackLoop::new();
        fb.record(10_000, 100, 0.0);
        fb.record(10_000, 100, -1.0);
        assert_eq!(fb.sample_count(), 0, "invalid samples should be ignored");
    }

    #[test]
    fn test_zero_dataset_size_ignored() {
        let fb = CboFeedbackLoop::new();
        fb.record(0, 100, 5.0);
        assert_eq!(fb.sample_count(), 0);
    }

    #[test]
    fn test_bounds_clamping() {
        let fb = CboFeedbackLoop::new();
        // Tiny latency → very small ratio → clamped to MIN
        for _ in 0..MIN_SAMPLES {
            fb.record(10_000, 100, 0.001);
        }
        let v = fb.adjusted_ms_per_cost_unit().unwrap();
        assert!(v >= MIN_MS_PER_UNIT, "should be clamped to minimum");
    }
}
