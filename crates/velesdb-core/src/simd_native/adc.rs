//! ADC (Asymmetric Distance Computation) for PQ-compressed vector search.
//!
//! Provides SIMD-accelerated distance computation using precomputed lookup tables.
//! Dispatches to AVX2 gather, NEON, or scalar path based on runtime detection.
//!
//! The crate-private API (`adc_distances_batch`) is called from the PQ
//! rescoring pipeline in `crate::quantization::pq::pq_adc_batch_rescore`, which
//! validates that every PQ code is `< k` (via `ProductQuantizer::validate_codes`)
//! before reaching the unsafe gather kernels here. That validation is the
//! precondition the `unsafe` blocks below rely on for in-bounds LUT indexing.

// The sole caller (`pq_adc_batch_rescore`) is persistence-gated, so all items
// in this module are dead when persistence is disabled.
#![cfg_attr(not(feature = "persistence"), allow(dead_code))]

#[allow(unused_imports)] // simd_level/SimdLevel used only on x86_64/aarch64 targets
use super::dispatch::{simd_level, SimdLevel};
#[cfg(target_arch = "x86_64")]
use super::reduction::hsum_avx256;

/// Compute ADC distances for a batch of PQ code vectors against a precomputed LUT.
///
/// # Arguments
///
/// * `lut` - Flat lookup table of shape `[m * k]`, indexed as `lut[subspace * k + code]`.
/// * `codes` - Slice of PQ code vectors; each inner slice has `m` entries (one centroid id per subspace).
/// * `m` - Number of subspaces.
///
/// # Returns
///
/// A vector of distances, one per code vector.
///
/// # Errors
///
/// Returns `Err` if `m` is zero or `lut.len()` is not divisible by `m`.
pub(crate) fn adc_distances_batch(
    lut: &[f32],
    codes: &[&[u16]],
    m: usize,
) -> crate::error::Result<Vec<f32>> {
    if m == 0 {
        return Err(crate::error::Error::InvalidVector(
            "ADC subspace count m must be > 0".into(),
        ));
    }
    if !lut.len().is_multiple_of(m) {
        return Err(crate::error::Error::InvalidVector(format!(
            "ADC lookup table length {} is not divisible by m={}",
            lut.len(),
            m
        )));
    }
    let k = lut.len() / m;

    Ok(match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 | SimdLevel::Avx512 => {
            // SAFETY: AVX2 ADC gather kernel requires CPU feature.
            // - Condition 1: `simd_level()` selected `Avx2` or `Avx512` after runtime detection.
            // SAFETY: call gather-based ADC kernel for higher throughput.
            codes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    // Prefetch next code vector into cache
                    if i + 1 < codes.len() {
                        super::prefetch::prefetch_vector_from_u16(codes[i + 1]);
                    }
                    // SAFETY: AVX2 ADC gather kernel — `simd_level()` confirmed Avx2/Avx512 above;
                    // codes were validated `< k` by `pq_adc_batch_rescore::validate_codes` (see
                    // module/function docs), so gather indices stay within `lut`.
                    unsafe { adc_single_avx2(lut, c, m, k) }
                })
                .collect()
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon => {
            // SAFETY: NEON ADC kernel requires aarch64 target.
            // - Condition 1: `simd_level()` selected `Neon` after runtime detection.
            // SAFETY: call NEON ADC kernel for higher throughput.
            codes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    // Prefetch next code vector into cache
                    if i + 1 < codes.len() {
                        super::prefetch::prefetch_vector_from_u16(codes[i + 1]);
                    }
                    // SAFETY: NEON is available (checked by `simd_level()` in outer match).
                    // - Condition 1: `simd_level()` selected `Neon`, confirming aarch64 NEON support.
                    // - Condition 2: codes were validated `< k` by
                    //   `pq_adc_batch_rescore::validate_codes` before this dispatch, so the
                    //   `get_unchecked` indices stay within `lut` (length `m * k`).
                    // SAFETY: Call per-code NEON ADC kernel for throughput inside the iterator.
                    unsafe { adc_single_neon(lut, c, m, k) }
                })
                .collect()
        }
        _ => adc_batch_scalar(lut, codes, m, k),
    })
}

/// Scalar ADC distance for a batch of code vectors.
fn adc_batch_scalar(lut: &[f32], codes: &[&[u16]], m: usize, k: usize) -> Vec<f32> {
    codes
        .iter()
        .map(|code| adc_single_scalar(lut, code, m, k))
        .collect()
}

/// Scalar ADC distance for a single code vector.
#[inline]
fn adc_single_scalar(lut: &[f32], code: &[u16], m: usize, k: usize) -> f32 {
    (0..m)
        .map(|subspace| {
            let idx = subspace * k + usize::from(code[subspace]);
            lut[idx]
        })
        .sum()
}

/// Build i32 index for one AVX2 lane: `(subspace * k + code[subspace])`.
///
/// PQ codebooks use m <= 64 and k <= 65535 (u16::MAX). The maximum index
/// value is 64 * 65535 + 65535 = 4_259_775, well within i32::MAX.
#[cfg(target_arch = "x86_64")]
#[inline]
fn lane_index(code: &[u16], subspace: usize, k: usize) -> i32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    let idx = (subspace * k + usize::from(code[subspace])) as i32;
    idx
}

/// AVX2 ADC distance using `_mm256_i32gather_ps` for 8 subspaces at a time.
///
/// # Safety
///
/// Preconditions (must be upheld by caller):
/// - CPU AVX2 feature must be available (verified by `simd_level()` dispatch).
/// - `code.len() == m`: every subspace must have an associated code entry.
/// - `usize::from(code[i]) < k` for all `i in 0..m`: each code must be a
///   valid centroid index so that `subspace * k + code[subspace]` stays
///   within the bounds of `lut` (length `m * k`).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn adc_single_avx2(lut: &[f32], code: &[u16], m: usize, k: usize) -> f32 {
    use std::arch::x86_64::{
        __m256, _mm256_add_ps, _mm256_i32gather_ps, _mm256_setr_epi32, _mm256_setzero_ps,
    };
    debug_assert_eq!(code.len(), m, "code length must equal m");
    debug_assert!(
        code.iter().all(|&c| usize::from(c) < k),
        "PQ code out of range: all codes must be < k ({k})"
    );

    let full_chunks = m / 8;

    let mut acc: __m256 = _mm256_setzero_ps();

    for chunk in 0..full_chunks {
        let base = chunk * 8;
        // SAFETY: `base + 0..7` are all < m because `chunk < full_chunks = m / 8`,
        // so `base + 7 = chunk * 8 + 7 < m`. All code values index into lut
        // which has size m * k, and each index = subspace * k + code[subspace]
        // where code[subspace] < k by PQ construction.
        let indices = _mm256_setr_epi32(
            lane_index(code, base, k),
            lane_index(code, base + 1, k),
            lane_index(code, base + 2, k),
            lane_index(code, base + 3, k),
            lane_index(code, base + 4, k),
            lane_index(code, base + 5, k),
            lane_index(code, base + 6, k),
            lane_index(code, base + 7, k),
        );

        // SAFETY: _mm256_i32gather_ps reads f32 values at base_ptr + index * scale.
        // - Scale = 4 = size_of::<f32>(), matching the f32 element type of `lut`.
        // - Each index is computed as subspace * k + code[subspace], validated to be
        //   within [0, m*k) which is within the `lut` slice bounds.
        // - All gathered reads are therefore within the allocated region of `lut`.
        // - No alignment requirement beyond f32 natural alignment (4 bytes) is imposed
        //   by gather instructions; `lut` is a &[f32] which guarantees f32 alignment.
        // - The pointer is valid for the duration of this intrinsic call (borrowed from `lut`).
        let gathered = _mm256_i32gather_ps::<4>(lut.as_ptr(), indices);
        acc = _mm256_add_ps(acc, gathered);
    }

    // Horizontal sum of acc
    let mut total = hsum_avx256(acc);

    // Handle tail subspaces (m % 8 != 0) with scalar loop.
    // `code` is indexed by subspace, so a range loop is the natural pattern here.
    #[allow(clippy::needless_range_loop)]
    for subspace in (full_chunks * 8)..m {
        let idx = subspace * k + usize::from(code[subspace]);
        total += lut[idx];
    }

    total
}

/// NEON ADC distance using 4-wide accumulation.
///
/// # Safety
///
/// Preconditions (must be upheld by caller):
/// - CPU NEON feature must be available on an aarch64 target (verified by
///   `simd_level()` dispatch).
/// - `code.len() == m`: every subspace must have an associated code entry.
/// - `usize::from(code[i]) < k` for all `i in 0..m`: each code must be a
///   valid centroid index so that `base * k + code[base]` stays within the
///   bounds of `lut` (length `m * k`), making the `get_unchecked` calls
///   well-defined.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[allow(clippy::wildcard_imports)] // idiomatic for NEON intrinsics
unsafe fn adc_single_neon(lut: &[f32], code: &[u16], m: usize, k: usize) -> f32 {
    use std::arch::aarch64::*;
    debug_assert_eq!(code.len(), m, "code length must equal m");
    debug_assert!(
        code.iter().all(|&c| usize::from(c) < k),
        "PQ code out of range: all codes must be < k ({k})"
    );

    let full_chunks = m / 4;
    let tail = m % 4;

    let mut acc = vdupq_n_f32(0.0);

    for chunk in 0..full_chunks {
        let base = chunk * 4;
        // SAFETY: `base + 0..3` are all < m (guaranteed by loop bound `chunk < m / 4`).
        // `code[base + i] < k` is verified by the `debug_assert!` at function entry,
        // so each index `(base + i) * k + code[base + i]` is within `lut` bounds.
        let vals: [f32; 4] = [
            *lut.get_unchecked((base) * k + usize::from(*code.get_unchecked(base))),
            *lut.get_unchecked((base + 1) * k + usize::from(*code.get_unchecked(base + 1))),
            *lut.get_unchecked((base + 2) * k + usize::from(*code.get_unchecked(base + 2))),
            *lut.get_unchecked((base + 3) * k + usize::from(*code.get_unchecked(base + 3))),
        ];
        let v = vld1q_f32(vals.as_ptr());
        acc = vaddq_f32(acc, v);
    }

    // Horizontal sum
    let mut total = vaddvq_f32(acc);

    // Handle tail subspaces with scalar loop
    let tail_start = full_chunks * 4;
    for (subspace, &codeword) in code.iter().enumerate().skip(tail_start).take(tail) {
        let idx = subspace * k + usize::from(codeword);
        total += lut[idx];
    }

    total
}

#[cfg(test)]
#[path = "adc_tests.rs"]
mod tests;
