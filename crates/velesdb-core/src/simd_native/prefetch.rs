//! CPU cache prefetch utilities for SIMD operations.
//!
//! Provides software prefetching hints to warm up CPU caches before
//! SIMD data access, reducing memory latency in batch operations.

/// L2 cache line size in bytes (standard for modern x86_64 CPUs).
pub const L2_CACHE_LINE_BYTES: usize = 64;

/// Calculates optimal prefetch distance based on vector dimension.
///
/// # Algorithm
///
/// Prefetch distance is computed to stay within L2 cache constraints:
/// - `distance = (vector_bytes / L2_CACHE_LINE).clamp(4, 16)`
/// - Minimum 4: Ensure enough lookahead for out-of-order execution
/// - Maximum 16: Prevent cache pollution from over-prefetching
#[inline]
#[must_use]
pub const fn calculate_prefetch_distance(dimension: usize) -> usize {
    let vector_bytes = dimension * std::mem::size_of::<f32>();
    let raw_distance = vector_bytes / L2_CACHE_LINE_BYTES;
    // Manual clamp for const fn
    if raw_distance < 4 {
        4
    } else if raw_distance > 16 {
        16
    } else {
        raw_distance
    }
}

/// Prefetches a vector into L1 cache (T0 hint) for upcoming SIMD operations.
///
/// # Platform Support
///
/// - **x86_64**: Uses `_mm_prefetch` with `_MM_HINT_T0`
/// - **aarch64**: Uses inline ASM workaround (rust-lang/rust#117217)
/// - **Other**: No-op (graceful degradation)
///
/// # Safety
///
/// This function is safe because prefetch instructions are hints and cannot
/// cause memory faults even with invalid addresses.
#[inline]
pub fn prefetch_vector(vector: &[f32]) {
    if vector.is_empty() {
        return;
    }

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: _mm_prefetch is a hint instruction that cannot cause memory faults.
        // - Condition 1: The pointer is derived from a valid slice reference (non-empty check above)
        // - Condition 2: Prefetch instructions are hints and never fault, even with invalid addresses
        // - Condition 3: x86_64 architecture guarantees _mm_prefetch availability
        // Reason: Software prefetching for cache optimization before SIMD data access.
        unsafe {
            use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            _mm_prefetch(vector.as_ptr().cast::<i8>(), _MM_HINT_T0);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::simd_neon_prefetch::prefetch_vector_neon(vector);
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = vector;
    }
}

/// Prefetches a `u16` slice into L1 cache (cross-platform).
///
/// Used by ADC batch operations to prefetch the next PQ code vector.
#[inline]
pub fn prefetch_vector_from_u16(data: &[u16]) {
    if data.is_empty() {
        return;
    }

    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: _mm_prefetch is a hint instruction that cannot cause memory faults.
        // - Condition 1: The pointer is derived from a valid slice reference.
        // - Condition 2: Prefetch hints never fault, even with invalid addresses.
        // Reason: Software prefetching for ADC code vectors before gather operations.
        unsafe {
            use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
            _mm_prefetch(data.as_ptr().cast::<i8>(), _MM_HINT_T0);
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::simd_neon_prefetch::prefetch_read_l1(data.as_ptr().cast::<u8>());
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = data;
    }
}

/// Prefetches a vector into multiple cache levels for larger vectors.
///
/// Coverage strategy per architecture:
/// - **`x86_64`**: 4 lines at 64B stride (offsets 0, 64, 128, 256)
/// - **`aarch64`**: 4 lines at 128B stride (Apple Silicon cache line size)
/// - **Other**: no-op
#[inline]
pub fn prefetch_vector_multi_cache_line(vector: &[f32]) {
    if vector.is_empty() {
        return;
    }

    #[cfg(target_arch = "x86_64")]
    {
        prefetch_multi_x86(vector);
    }

    #[cfg(target_arch = "aarch64")]
    {
        prefetch_multi_arm64(vector);
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = vector;
    }
}

/// x86_64 multi-cache-line prefetch at 64B stride.
#[cfg(target_arch = "x86_64")]
#[inline]
fn prefetch_multi_x86(vector: &[f32]) {
    use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0, _MM_HINT_T1, _MM_HINT_T2};

    let vector_bytes = std::mem::size_of_val(vector);

    // SAFETY: _mm_prefetch is a non-faulting hint instruction.
    // - Condition 1: Pointers derived from valid slice reference.
    // - Condition 2: Offsets checked against vector_bytes.
    // Reason: Multi-level cache warming for large vectors before SIMD processing.
    unsafe {
        _mm_prefetch(vector.as_ptr().cast::<i8>(), _MM_HINT_T0);

        if vector_bytes > L2_CACHE_LINE_BYTES {
            _mm_prefetch(
                vector.as_ptr().cast::<i8>().add(L2_CACHE_LINE_BYTES),
                _MM_HINT_T1,
            );
        }
        if vector_bytes > L2_CACHE_LINE_BYTES * 2 {
            _mm_prefetch(
                vector.as_ptr().cast::<i8>().add(L2_CACHE_LINE_BYTES * 2),
                _MM_HINT_T2,
            );
        }
        if vector_bytes > L2_CACHE_LINE_BYTES * 4 {
            _mm_prefetch(
                vector.as_ptr().cast::<i8>().add(L2_CACHE_LINE_BYTES * 4),
                _MM_HINT_T2,
            );
        }
    }
}

/// ARM64 multi-cache-line prefetch at 128B stride (Apple Silicon cache line size).
///
/// Apple M1-M4 use 128-byte cache lines. Graviton uses 64B but a 128B stride
/// still provides useful lookahead. Prefetches 4 lines for vectors > 512B.
#[cfg(target_arch = "aarch64")]
#[inline]
fn prefetch_multi_arm64(vector: &[f32]) {
    const ARM_CL: usize = 128;
    let base = vector.as_ptr().cast::<u8>();
    let vector_bytes = std::mem::size_of_val(vector);

    // Line 0 → L1
    crate::simd_neon_prefetch::prefetch_read_l1(base);

    // Line 1 → L1 (offset 128B)
    if vector_bytes > ARM_CL {
        // SAFETY: Prefetch is a non-faulting hint; offset < vector_bytes.
        let ptr = unsafe { base.add(ARM_CL) };
        crate::simd_neon_prefetch::prefetch_read_l1(ptr);
    }
    // Line 2 → L2 (offset 256B)
    if vector_bytes > ARM_CL * 2 {
        // SAFETY: Same; offset 256 < vector_bytes.
        let ptr = unsafe { base.add(ARM_CL * 2) };
        crate::simd_neon_prefetch::prefetch_read_l2(ptr);
    }
    // Line 3 → L3 (offset 512B)
    if vector_bytes > ARM_CL * 4 {
        // SAFETY: Same; offset 512 < vector_bytes.
        let ptr = unsafe { base.add(ARM_CL * 4) };
        crate::simd_neon_prefetch::prefetch_read_l3(ptr);
    }
}
