//! Cross-implementation quantization parity tests (#1543).
//!
//! The core-parity audit found that the WASM SQ8/Binary quantization paths
//! (`store_insert::encode_sq8`, `store_insert::encode_binary`,
//! `vector_ops::ScratchBuffer::decode_sq8`, `store_get::decode_sq8`) could
//! diverge from `velesdb_core::{QuantizedVector, BinaryQuantizedVector}`:
//!
//! - SQ8 used an ad hoc `1e-10` degenerate-range epsilon instead of core's
//!   `f32::EPSILON`, so a near-constant vector could quantize as "normal" in
//!   WASM and "degenerate" in core.
//! - The degenerate fallback itself differed: core fills every dimension
//!   with byte 128 (`QuantizedVector::from_f32`); the old WASM formula
//!   collapsed to byte 0 for every dimension.
//! - Binary quantization parity with `BinaryQuantizedVector::from_f32` was
//!   asserted only in a comment, never a test.
//!
//! These tests assert byte-for-byte equality of the *quantized data* against
//! core on a fixed dataset that specifically includes the regression range
//! (a vector whose min/max gap sits between the old `1e-10` WASM threshold
//! and core's `f32::EPSILON` threshold), plus other degenerate/extreme edges.
//! `test_sq8_restore_matches_core_within_float_tolerance` additionally checks
//! dequantized (`to_f32`) values match core within a tight tolerance — not
//! bit-for-bit, because the WASM on-disk/`IndexedDB` persistence format
//! (`serialization.rs` v2) stores an *encode* scale (`255.0 / range`) rather
//! than core's `(min, max)` pair, so decode divides by that scale instead of
//! multiplying by core's independently-rounded `range / 255.0`. Changing the
//! stored representation to guarantee bit-identical decode would change the
//! v2 on-disk format and silently corrupt already-persisted `IndexedDB`
//! stores from older WASM builds with no version bump — out of scope for
//! this fix. See the PR description for the full trade-off.
//!
//! Host-run (`#[test]`, native target): these run wherever
//! `cargo test -p velesdb-wasm` runs, which already includes this crate via
//! `cargo test --workspace` in the `Tests` CI job, and are additionally
//! invoked as an explicitly named step in the `WASM Build Check` job
//! (`.github/workflows/ci.yml`) for a dedicated, visible parity gate.
//!
//! TDD: before the #1543 fix, `test_sq8_quantization_matches_core_byte_for_byte`
//! and `test_sq8_epsilon_regression_range_matches_core_degenerate_fallback`
//! failed (RED) on the `near_constant_regression` and `constant` cases; after
//! the fix all cases pass (GREEN).
//!
//! ## Follow-up: non-finite range (Fable review)
//!
//! The initial fix's `scale == 0.0` degenerate-range decode sentinel was not
//! hermetic: a vector whose range overflows to `+Infinity` (finite min/max
//! whose difference overflows, or a literal `+/-Infinity` element) also
//! computes `scale = 255.0 / range = 0.0` — through the *normal* branch, not
//! the degenerate one — colliding with the sentinel. `test_sq8_non_finite_range_matches_core_exactly`
//! and the `range_*` dataset cases pin this down; they were RED (WASM decoded
//! to `min`, core decodes to `NaN`) before `store_insert::encode_sq8` grew a
//! dedicated, empirically-verified-against-core `!range.is_finite()` branch
//! (all-zero encoded bytes, `f32::NAN` decode sentinel) ahead of the
//! `range < f32::EPSILON` check.

use crate::store_get::get_vector_at_index;
use crate::store_insert::insert_vector;
use crate::store_new::create_store;
use crate::{DistanceMetric, StorageMode};
use velesdb_core::{BinaryQuantizedVector, QuantizedVector};

/// Fixed parity dataset: `(case name, vector)`. Covers the normal path plus
/// every degenerate/extreme edge called out by the #1543 audit.
fn parity_dataset() -> Vec<(&'static str, Vec<f32>)> {
    vec![
        ("normal", vec![0.3, -1.2, 4.5, -0.001, 2.7]),
        ("constant", vec![0.5, 0.5, 0.5, 0.5, 0.5, 0.5]),
        // Range = 8e-8: above the OLD WASM threshold (`(max-min).abs() < 1e-10`,
        // so the old code took the "normal" branch) but below core's
        // `f32::EPSILON` (~1.1920929e-7, so core takes the degenerate
        // branch). This is the exact divergence #1543 reports. Values are
        // kept near zero magnitude so f32 has enough precision to represent
        // an 8e-8 gap distinctly (near 1.0 the float spacing is itself
        // `f32::EPSILON`, too coarse to express this gap at all).
        (
            "near_constant_regression",
            vec![0.0, 5.0e-8, -3.0e-8, 2.0e-8, 1.0e-8],
        ),
        (
            "extreme_magnitude",
            vec![1.0e30, -1.0e30, 5.0e29, -2.5e29, 0.0, 3.0],
        ),
        ("with_nan", vec![1.0, f32::NAN, -3.0, 2.5, 0.0]),
        ("zero_and_neg_zero", vec![0.0, -0.0, 0.0, 0.0, 0.0]),
        ("not_multiple_of_8_dims", vec![1.0, -1.0, 1.0, -1.0, 1.0]),
        // Non-finite range (Fable review, #1543 follow-up): min/max are
        // finite but `max - min` overflows to +Infinity in f32. This is
        // NOT the `range < f32::EPSILON` degenerate case (`+inf` is never
        // `< EPSILON`) — core's `QuantizedVector::from_f32` takes the
        // *normal* branch here, computes `scale = 255.0 / range = 0.0`,
        // and every per-element `(v - min) * 0.0` term evaluates to
        // exactly `0.0` (finite operand) or `NaN` (an operand that itself
        // overflowed to infinity), which `round().clamp().cast()` always
        // turns into byte 0. Confirmed empirically against core: encoded
        // data is uniformly all-zero, `to_f32()` is uniformly all-NaN
        // (`q * (range/255.0) + min` = `0.0 * inf + min` = NaN).
        (
            "range_overflow_finite_minmax",
            vec![-f32::MAX, f32::MAX, 0.0, 1.0],
        ),
        // A literal +/-Infinity element: min stays finite, max becomes
        // +inf (or vice versa), range is still +inf — same core behavior
        // as the overflow case above (all-zero data, all-NaN restore).
        (
            "range_infinite_literal_element",
            vec![1.0, f32::INFINITY, -3.0, 2.0],
        ),
        // Every element is NaN or +Infinity: min/max fold (NaN-ignoring)
        // both settle on +inf, so range = inf - inf = NaN (not +inf, but
        // still never `< f32::EPSILON`, so core again takes the "normal"
        // branch). Empirically confirmed identical outcome to the +inf
        // cases: all-zero data, all-NaN restore.
        (
            "range_nan_all_inf_or_nan",
            vec![f32::NAN, f32::INFINITY, f32::NAN, f32::INFINITY],
        ),
    ]
}

/// True when both values are `NaN`, or both are within `tol` of each other.
/// Plain `(a - b).abs() <= tol` is always false when either side is `NaN`
/// (IEEE 754), which would wrongly fail restore-parity checks for the
/// non-finite-range dataset cases where *both* core and WASM legitimately
/// restore to `NaN`.
fn nan_aware_close(a: f32, b: f32, tol: f32) -> bool {
    (a.is_nan() && b.is_nan()) || (a - b).abs() <= tol
}

/// Encodes `vector` through the real WASM `SQ8` insert path and returns
/// `(quantized_bytes, min, scale)` for the single inserted vector.
fn wasm_encode_sq8(vector: &[f32]) -> (Vec<u8>, f32, f32) {
    let mut store = create_store(vector.len(), DistanceMetric::Euclidean, StorageMode::SQ8);
    insert_vector(&mut store, 1, vector);
    (
        store.data_sq8.clone(),
        store.sq8_mins[0],
        store.sq8_scales[0],
    )
}

/// Encodes `vector` through the real WASM `Binary` insert path and returns
/// the packed bytes for the single inserted vector.
fn wasm_encode_binary(vector: &[f32]) -> Vec<u8> {
    let mut store = create_store(vector.len(), DistanceMetric::Hamming, StorageMode::Binary);
    insert_vector(&mut store, 1, vector);
    store.data_binary.clone()
}

#[test]
fn test_sq8_quantization_matches_core_byte_for_byte() {
    for (name, vector) in parity_dataset() {
        let core_q = QuantizedVector::from_f32(&vector);
        let (wasm_bytes, wasm_min, _wasm_scale) = wasm_encode_sq8(&vector);

        assert_eq!(
            wasm_bytes, core_q.data,
            "SQ8 quantized bytes diverge from core for case `{name}` (vector={vector:?})"
        );
        assert_eq!(
            wasm_min, core_q.min,
            "SQ8 min diverges from core for case `{name}`"
        );
    }
}

#[test]
fn test_sq8_epsilon_regression_range_matches_core_degenerate_fallback() {
    // Pinned regression case: range = 8e-8, strictly between the old WASM
    // epsilon (1e-10) and core's threshold (f32::EPSILON). Before #1543's
    // fix this vector quantized as "normal" in WASM (bytes derived from the
    // per-element formula) but as "degenerate" in core (every byte = 128).
    let vector = vec![0.0_f32, 5.0e-8, -3.0e-8, 2.0e-8, 1.0e-8];
    let core_q = QuantizedVector::from_f32(&vector);
    let (wasm_bytes, wasm_min, wasm_scale) = wasm_encode_sq8(&vector);

    assert_eq!(
        core_q.data,
        vec![128u8; vector.len()],
        "sanity: core takes the degenerate branch"
    );
    assert_eq!(
        wasm_bytes,
        vec![128u8; vector.len()],
        "WASM must also take the degenerate branch"
    );
    assert_eq!(wasm_bytes, core_q.data);
    assert_eq!(wasm_min, core_q.min);
    assert_eq!(
        wasm_scale, 0.0,
        "degenerate range must encode the scale=0.0 sentinel"
    );
}

#[test]
fn test_binary_quantization_matches_core_byte_for_byte() {
    for (name, vector) in parity_dataset() {
        let core_b = BinaryQuantizedVector::from_f32(&vector);
        let wasm_bytes = wasm_encode_binary(&vector);

        assert_eq!(
            wasm_bytes, core_b.data,
            "Binary quantized bytes diverge from core for case `{name}` (vector={vector:?})"
        );
        assert_eq!(
            wasm_bytes.len(),
            vector.len().div_ceil(8),
            "packed byte count mismatch for case `{name}`"
        );
    }
}

#[test]
fn test_sq8_restore_matches_core_within_float_tolerance() {
    for (name, vector) in parity_dataset() {
        let core_q = QuantizedVector::from_f32(&vector);
        let core_restored = core_q.to_f32();

        let mut store = create_store(vector.len(), DistanceMetric::Euclidean, StorageMode::SQ8);
        insert_vector(&mut store, 1, &vector);
        let wasm_restored = get_vector_at_index(&store, 0);

        let range = (core_q.max - core_q.min).abs();
        // Tolerance covers only the residual float-rounding-path difference
        // between WASM's `q / encode_scale` and core's `q * decode_scale`
        // (see module doc) — not quantization error, since both decode the
        // same quantized byte.
        let tol = (range * 1.0e-5).max(1.0e-4);

        assert_eq!(
            wasm_restored.len(),
            core_restored.len(),
            "restored dimension mismatch for case `{name}`"
        );
        for (i, (&w, &c)) in wasm_restored.iter().zip(core_restored.iter()).enumerate() {
            assert!(
                nan_aware_close(w, c, tol),
                "restore diverges for case `{name}` dim {i}: wasm={w} core={c} tol={tol}"
            );
        }
    }
}

#[test]
fn test_sq8_non_finite_range_matches_core_exactly() {
    // Fable review follow-up on #1543: the `scale == 0.0` degenerate-range
    // sentinel (see `store_insert::encode_sq8`) is not hermetic — a
    // genuinely non-degenerate vector whose range overflows to `+Infinity`
    // also computes `scale = 255.0 / range = 0.0` through the *normal*
    // branch, colliding with the sentinel. Before the fix, WASM's decode
    // then wrongly took the "degenerate" path and returned `min` for every
    // dimension, where core actually returns `NaN` for every dimension
    // (`to_f32`'s `q * (range/255.0) + min` with `range = +inf`).
    for (name, vector) in [
        (
            "range_overflow_finite_minmax",
            vec![-f32::MAX, f32::MAX, 0.0, 1.0],
        ),
        (
            "range_infinite_literal_element",
            vec![1.0, f32::INFINITY, -3.0, 2.0],
        ),
        (
            "range_nan_all_inf_or_nan",
            vec![f32::NAN, f32::INFINITY, f32::NAN, f32::INFINITY],
        ),
    ] {
        let core_q = QuantizedVector::from_f32(&vector);
        let core_restored = core_q.to_f32();
        assert_eq!(
            core_q.data,
            vec![0u8; vector.len()],
            "sanity: core's own encode is all-zero bytes for case `{name}`"
        );
        assert!(
            core_restored.iter().all(|v| v.is_nan()),
            "sanity: core's own restore is all-NaN for case `{name}`"
        );

        let (wasm_bytes, _wasm_min, _wasm_scale) = wasm_encode_sq8(&vector);
        assert_eq!(
            wasm_bytes, core_q.data,
            "SQ8 encoded bytes diverge from core for non-finite-range case `{name}`"
        );

        let mut store = create_store(vector.len(), DistanceMetric::Euclidean, StorageMode::SQ8);
        insert_vector(&mut store, 1, &vector);
        let wasm_restored = get_vector_at_index(&store, 0);
        assert!(
            wasm_restored.iter().all(|v| v.is_nan()),
            "WASM restore must be all-NaN (matching core) for case `{name}`, got {wasm_restored:?}"
        );
    }
}
