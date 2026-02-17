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

/// Prefetches a vector into multiple cache levels for larger vectors.
///
/// Uses different cache level hints:
/// - First cache line -> L1 (T0 hint)
/// - Second cache line -> L2 (T1 hint)
/// - Third+ cache lines -> L3 (T2 hint)
#[inline]
pub fn prefetch_vector_multi_cache_line(vector: &[f32]) {
    if vector.is_empty() {
        return;
    }

    let vector_bytes = std::mem::size_of_val(vector);

    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_prefetch, _MM_HINT_T0, _MM_HINT_T1, _MM_HINT_T2};

        // SAFETY: _mm_prefetch is a hint instruction that cannot cause memory faults.
        // - Condition 1: All pointers are derived from a valid slice reference (non-empty check above)
        // - Condition 2: Prefetch instructions are hints and never fault, even with invalid addresses
        // - Condition 3: Pointer arithmetic stays within bounds (offsets checked against vector_bytes)
        // - Condition 4: x86_64 architecture guarantees _mm_prefetch availability
        // Reason: Multi-level cache prefetching for large vectors before SIMD processing.
        unsafe {
            _mm_prefetch(vector.as_ptr().cast::<i8>(), _MM_HINT_T0);

            if vector_bytes > L2_CACHE_LINE_BYTES {
                let ptr = vector.as_ptr().cast::<i8>().add(L2_CACHE_LINE_BYTES);
                _mm_prefetch(ptr, _MM_HINT_T1);
            }

            if vector_bytes > L2_CACHE_LINE_BYTES * 2 {
                let ptr = vector.as_ptr().cast::<i8>().add(L2_CACHE_LINE_BYTES * 2);
                _mm_prefetch(ptr, _MM_HINT_T2);
            }

            if vector_bytes > L2_CACHE_LINE_BYTES * 4 {
                let ptr = vector.as_ptr().cast::<i8>().add(L2_CACHE_LINE_BYTES * 4);
                _mm_prefetch(ptr, _MM_HINT_T2);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::simd_neon_prefetch::prefetch_vector_neon(vector);
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = vector;
        let _ = vector_bytes;
    }
}
