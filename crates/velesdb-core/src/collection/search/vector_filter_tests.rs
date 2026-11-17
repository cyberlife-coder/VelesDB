//! Unit tests for `vector_filter` oversampling computation.

use super::vector_filter::{compute_oversampled_k, ef_to_quality};
use crate::filter::{Condition, Filter};
use crate::SearchQuality;

fn eq_filter() -> Filter {
    Filter::new(Condition::Eq {
        field: "category".to_string(),
        value: serde_json::json!("science"),
    })
}

/// Regression: with `k >= 10_000` the lower bound `(k + 10)` used to exceed
/// the `10_000` upper cap and `f64::clamp` panicked with `min > max`; then the
/// budget saturated at the cap, below `k`, so a search for more than 10 000
/// points returned 10 000. The ceiling bounds the oversampling, not `k`.
#[test]
fn test_compute_oversampled_k_never_below_k() {
    assert_eq!(compute_oversampled_k(100_000, &eq_filter(), None), 100_010);
    assert_eq!(
        compute_oversampled_k(usize::MAX, &eq_filter(), None),
        usize::MAX
    );
}

/// Exact boundary: at k = 9_990 the lower bound (k + 10) equals the cap.
#[test]
fn test_compute_oversampled_k_boundary_at_cap() {
    assert_eq!(compute_oversampled_k(9_990, &eq_filter(), None), 10_000);
    assert_eq!(compute_oversampled_k(10_000, &eq_filter(), None), 10_010);
}

/// Nominal: small k keeps the selectivity-driven oversampling
/// (`k / 0.1` for an Eq filter), bounded below by `k + 10`.
#[test]
fn test_compute_oversampled_k_small_k_unchanged() {
    // selectivity(Eq) = 0.1 -> 100 / 0.1 = 1000.
    assert_eq!(compute_oversampled_k(100, &eq_filter(), None), 1_000);
    // 5 / 0.1 = 50, above the lower bound (15).
    assert_eq!(compute_oversampled_k(5, &eq_filter(), None), 50);
}

/// `WITH (ef_search = N)` must honor the exact budget verbatim, not snap to a
/// coarse named profile. The old bracket would map 200 to Accurate and 64 to
/// Fast; the value-preserving mapping returns `Custom(N)` for both.
#[test]
fn test_ef_to_quality_preserves_exact_value() {
    assert_eq!(ef_to_quality(200), SearchQuality::Custom(200));
    // 64 is the old Fast/Balanced bracket edge: still preserved exactly.
    assert_eq!(ef_to_quality(64), SearchQuality::Custom(64));
}
