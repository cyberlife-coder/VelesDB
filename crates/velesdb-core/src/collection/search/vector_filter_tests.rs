//! Unit tests for `vector_filter` oversampling computation.

use super::vector_filter::compute_oversampled_k;
use crate::filter::{Condition, Filter};

fn eq_filter() -> Filter {
    Filter::new(Condition::Eq {
        field: "category".to_string(),
        value: serde_json::json!("science"),
    })
}

/// Regression: with `k >= 10_000` the lower bound `(k + 10)` used to exceed
/// the `10_000` upper cap and `f64::clamp` panicked with `min > max`.
/// The candidate budget must instead saturate at the cap.
#[test]
fn test_compute_oversampled_k_never_panics_at_max_limit() {
    let k = 100_000;
    let candidates = compute_oversampled_k(k, &eq_filter());
    assert_eq!(candidates, 10_000, "budget saturates at the 10_000 cap");
}

/// Exact boundary: at k = 9_990 the lower bound (k + 10) equals the cap.
#[test]
fn test_compute_oversampled_k_boundary_at_cap() {
    assert_eq!(compute_oversampled_k(9_990, &eq_filter()), 10_000);
    assert_eq!(compute_oversampled_k(10_000, &eq_filter()), 10_000);
}

/// Nominal: small k keeps the selectivity-driven oversampling
/// (`k / 0.1` for an Eq filter), bounded below by `k + 10`.
#[test]
fn test_compute_oversampled_k_small_k_unchanged() {
    // selectivity(Eq) = 0.1 -> 100 / 0.1 = 1000.
    assert_eq!(compute_oversampled_k(100, &eq_filter()), 1_000);
    // 5 / 0.1 = 50, above the lower bound (15).
    assert_eq!(compute_oversampled_k(5, &eq_filter()), 50);
}
