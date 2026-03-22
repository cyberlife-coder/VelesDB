//! ADC (Asymmetric Distance Computation) for PQ-compressed vector search.
//!
//! Provides SIMD-accelerated distance computation using precomputed lookup tables.
//! Dispatches to AVX2 gather, NEON, or scalar path based on runtime detection.
//!
//! The public-crate API (`adc_distances_batch`) is wired into the PQ search
//! pipeline in Phase 03. Functions are marked `#[allow(dead_code)]` until then.

// `adc_distances_batch` is the Phase 03 search hot-path; suppressed until wired into
// the PQ rescoring pipeline. Remove this allow when Phase 03 integration is complete.
#![allow(dead_code)]

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
/// # Panics
///
/// Panics if `m` is zero or `lut.len()` is not divisible by `m`.
#[must_use]
pub(crate) fn adc_distances_batch(lut: &[f32], codes: &[&[u16]], m: usize) -> Vec<f32> {
    assert!(m > 0, "m must be > 0");
    assert!(lut.len() % m == 0, "lut length must be divisible by m");
    let k = lut.len() / m;

    match simd_level() {
        #[cfg(target_arch = "x86_64")]
        SimdLevel::Avx2 | SimdLevel::Avx512 => {
            // SAFETY: AVX2 ADC gather kernel requires CPU feature.
            // - Condition 1: `simd_level()` selected `Avx2` or `Avx512` after runtime detection.
            // Reason: call gather-based ADC kernel for higher throughput.
            codes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    // Prefetch next code vector into cache
                    if i + 1 < codes.len() {
                        super::prefetch::prefetch_vector_from_u16(codes[i + 1]);
                    }
                    // SAFETY: AVX2 ADC gather kernel — `simd_level()` confirmed Avx2/Avx512 above.
                    unsafe { adc_single_avx2(lut, c, m, k) }
                })
                .collect()
        }
        #[cfg(target_arch = "aarch64")]
        SimdLevel::Neon => {
            // SAFETY: NEON ADC kernel requires aarch64 target.
            // - Condition 1: `simd_level()` selected `Neon` after runtime detection.
            // Reason: call NEON ADC kernel for higher throughput.
            codes
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    // Prefetch next code vector into cache
                    if i + 1 < codes.len() {
                        super::prefetch::prefetch_vector_from_u16(codes[i + 1]);
                    }
                    unsafe { adc_single_neon(lut, c, m, k) }
                })
                .collect()
        }
        _ => adc_batch_scalar(lut, codes, m, k),
    }
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
    for subspace in tail_start..tail_start + tail {
        let idx = subspace * k + usize::from(code[subspace]);
        total += lut[idx];
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build a simple LUT and codes for testing.
    /// m subspaces, k centroids, `LUT[s*k + c] = (s * k + c)` as `f32`.
    fn make_sequential_lut(m: usize, k: usize) -> Vec<f32> {
        (0..m * k)
            .map(|i| {
                #[allow(clippy::cast_precision_loss)]
                let v = i as f32;
                v
            })
            .collect()
    }

    #[test]
    fn adc_scalar_correct_sum() {
        // m=4, k=4, codes=[0,1,2,3]
        // Expected: lut[0*4+0] + lut[1*4+1] + lut[2*4+2] + lut[3*4+3]
        //         = 0 + 5 + 10 + 15 = 30
        let m = 4;
        let k = 4;
        let lut = make_sequential_lut(m, k);
        let codes: Vec<u16> = vec![0, 1, 2, 3];
        let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];
        let result = adc_distances_batch(&lut, &codes_ref, m);
        assert_eq!(result.len(), 1);
        assert!(
            (result[0] - 30.0).abs() < 1e-6,
            "expected 30.0, got {}",
            result[0]
        );
    }

    #[test]
    fn adc_batch_multiple_codes() {
        let m = 2;
        let k = 4;
        let lut = make_sequential_lut(m, k);
        // code1=[0,0]: lut[0]+lut[4] = 0+4 = 4
        // code2=[3,3]: lut[3]+lut[7] = 3+7 = 10
        let c1: Vec<u16> = vec![0, 0];
        let c2: Vec<u16> = vec![3, 3];
        let codes_ref: Vec<&[u16]> = vec![c1.as_slice(), c2.as_slice()];
        let result = adc_distances_batch(&lut, &codes_ref, m);
        assert_eq!(result.len(), 2);
        assert!((result[0] - 4.0).abs() < 1e-6);
        assert!((result[1] - 10.0).abs() < 1e-6);
    }

    #[test]
    fn adc_m8_k256_standard_config() {
        let m = 8;
        let k = 256;
        let lut = make_sequential_lut(m, k);
        // codes = [0, 0, 0, 0, 0, 0, 0, 0]
        // Expected: sum of lut[s*256+0] for s=0..7 = 0+256+512+768+1024+1280+1536+1792 = 7168
        let codes: Vec<u16> = vec![0; 8];
        let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];
        let result = adc_distances_batch(&lut, &codes_ref, m);
        assert!(
            (result[0] - 7168.0).abs() < 1e-2,
            "expected 7168.0, got {}",
            result[0]
        );
    }

    #[test]
    fn adc_m_not_divisible_by_8() {
        // m=5 (not divisible by 8), k=4
        let m = 5;
        let k = 4;
        let lut = make_sequential_lut(m, k);
        // codes = [1, 1, 1, 1, 1]
        // Expected: lut[1] + lut[5] + lut[9] + lut[13] + lut[17] = 1+5+9+13+17 = 45
        let codes: Vec<u16> = vec![1, 1, 1, 1, 1];
        let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];
        let result = adc_distances_batch(&lut, &codes_ref, m);
        assert!(
            (result[0] - 45.0).abs() < 1e-6,
            "expected 45.0, got {}",
            result[0]
        );
    }

    #[test]
    fn adc_lut_size_m8_k256() {
        let m = 8;
        let k = 256;
        let lut = make_sequential_lut(m, k);
        // 8 * 256 * 4 bytes = 8192 bytes = 8KB
        assert_eq!(lut.len() * std::mem::size_of::<f32>(), 8192);
    }

    #[test]
    fn adc_avx2_matches_scalar() {
        // Compare SIMD path against scalar for m=8, k=16
        let m = 8;
        let k = 16;
        let lut = make_sequential_lut(m, k);
        let codes: Vec<u16> = vec![3, 7, 1, 15, 0, 8, 12, 5];
        let codes_ref: Vec<&[u16]> = vec![codes.as_slice()];

        // Scalar reference
        let scalar_result = adc_batch_scalar(&lut, &codes_ref, m, k);
        // Dispatch (may use AVX2 or scalar depending on platform)
        let dispatch_result = adc_distances_batch(&lut, &codes_ref, m);

        assert!(
            (scalar_result[0] - dispatch_result[0]).abs() < 1e-4,
            "SIMD dispatch ({}) != scalar ({}) beyond f32 epsilon",
            dispatch_result[0],
            scalar_result[0]
        );
    }
}
