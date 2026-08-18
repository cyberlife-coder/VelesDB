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
        (MIN_MS_PER_UNIT..=MAX_MS_PER_UNIT).contains(&v),
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

#[test]
fn test_large_value_clamped_before_cast() {
    let fb = CboFeedbackLoop::new();
    // A ratio much larger than MAX_MS_PER_UNIT must be clamped before the
    // u64 cast so we never produce an overflowed or truncated scaled value.
    for _ in 0..MIN_SAMPLES {
        fb.record(1, 1, 1e10);
    }
    if let Some(v) = fb.adjusted_ms_per_cost_unit() {
        assert!(
            v <= MAX_MS_PER_UNIT,
            "value must be clamped to MAX_MS_PER_UNIT, got {v}"
        );
    }
}
