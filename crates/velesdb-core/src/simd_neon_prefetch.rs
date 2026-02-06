//! ARM64 NEON prefetch operations via inline assembly (EPIC-054 US-002).
//!
//! This module provides prefetch hints for ARM64 targets using inline assembly,
//! bypassing the unstable `stdarch_aarch64_prefetch` intrinsic (rust#117217).
//!
//! # Performance Impact
//!
//! Prefetching can improve HNSW search performance by 10-20% by hiding memory
//! latency when traversing graph neighbors.
//!
//! # Prefetch Locality Hints
//!
//! - **L1**: Prefetch into L1 cache (fastest, smallest)
//! - **L2**: Prefetch into L2 cache (medium speed, larger)
//! - **L3**: Prefetch into L3 cache (slowest, largest)
//!
//! # Example
//!
//! ```ignore
//! use velesdb_core::simd_neon_prefetch::prefetch_read_l1;
//!
//! let data = vec![0.0f32; 1024];
//! // Prefetch upcoming data before accessing it
//! prefetch_read_l1(data.as_ptr() as *const u8);
//! ```

/// Prefetch data for reading into L1 cache.
///
/// This function issues a `PRFM PLDL1KEEP` instruction on ARM64.
///
/// # Safety
///
/// This function is safe because prefetch instructions are hints and
/// do not cause faults on invalid addresses.
///
/// # Arguments
///
/// * `ptr` - Pointer to the data to prefetch
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn prefetch_read_l1(ptr: *const u8) {
    // SAFETY: Prefetch instructions are hints and cannot cause faults.
    // Even if ptr is invalid, the CPU will simply ignore the hint.
    unsafe {
        core::arch::asm!(
            "prfm pldl1keep, [{ptr}]",
            ptr = in(reg) ptr,
            options(nostack, preserves_flags)
        );
    }
}

/// Prefetch data for reading into L2 cache.
///
/// Use this for data that will be accessed soon but not immediately.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn prefetch_read_l2(ptr: *const u8) {
    // SAFETY: `asm!(prfm ...)` emits a prefetch hint only.
    // - Condition 1: `prfm` does not dereference memory architecturally.
    // - Condition 2: `nostack` and `preserves_flags` match instruction behavior.
    // Reason: Stable Rust lacks a fully-stable aarch64 prefetch intrinsic.
    unsafe {
        core::arch::asm!(
            "prfm pldl2keep, [{ptr}]",
            ptr = in(reg) ptr,
            options(nostack, preserves_flags)
        );
    }
}

/// Prefetch data for reading into L3 cache.
///
/// Use this for data that will be accessed later in the computation.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn prefetch_read_l3(ptr: *const u8) {
    // SAFETY: `asm!(prfm ...)` emits a prefetch hint only.
    // - Condition 1: Invalid pointers are tolerated by hardware for prefetch hints.
    // - Condition 2: The instruction does not write through `ptr`.
    // Reason: We need explicit L3-prefetch locality selection on stable.
    unsafe {
        core::arch::asm!(
            "prfm pldl3keep, [{ptr}]",
            ptr = in(reg) ptr,
            options(nostack, preserves_flags)
        );
    }
}

/// Prefetch data for writing into L1 cache.
///
/// Use this when you know you'll be writing to the data soon.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn prefetch_write_l1(ptr: *const u8) {
    // SAFETY: `asm!(prfm ...)` emits a write-prefetch hint only.
    // - Condition 1: This does not perform a memory store.
    // - Condition 2: Calling convention is preserved by `nostack` and `preserves_flags`.
    // Reason: We need explicit store-intent prefetch on aarch64.
    unsafe {
        core::arch::asm!(
            "prfm pstl1keep, [{ptr}]",
            ptr = in(reg) ptr,
            options(nostack, preserves_flags)
        );
    }
}

/// Prefetch a vector (f32 slice) for reading into L1 cache.
///
/// This is the main function used in HNSW traversal to prefetch
/// upcoming candidate vectors.
///
/// # Arguments
///
/// * `vector` - The f32 slice to prefetch
///
/// # Performance Notes
///
/// - Each cache line is typically 64 bytes (16 f32 values)
/// - For 768D vectors, this prefetches the first cache line
/// - Call multiple times with offset for larger vectors
#[cfg(target_arch = "aarch64")]
#[inline(always)]
pub fn prefetch_vector_neon(vector: &[f32]) {
    if !vector.is_empty() {
        prefetch_read_l1(vector.as_ptr() as *const u8);

        // Prefetch additional cache lines for larger vectors
        let cache_line_size = 64; // bytes
        let vector_bytes = vector.len() * std::mem::size_of::<f32>();

        if vector_bytes > cache_line_size {
            // Prefetch second cache line
            // SAFETY: Pointer arithmetic stays within `vector` because this branch
            // only runs when at least one full extra cache line exists.
            let ptr = unsafe { (vector.as_ptr() as *const u8).add(cache_line_size) };
            prefetch_read_l2(ptr);
        }

        if vector_bytes > cache_line_size * 2 {
            // Prefetch third cache line into L3
            // SAFETY: Pointer arithmetic stays within `vector` because this branch
            // only runs when at least two full extra cache lines exist.
            let ptr = unsafe { (vector.as_ptr() as *const u8).add(cache_line_size * 2) };
            prefetch_read_l3(ptr);
        }
    }
}

/// Calculate optimal prefetch distance based on vector dimension.
///
/// Returns the number of vectors to prefetch ahead during HNSW traversal.
///
/// # Formula
///
/// The prefetch distance is calculated to hide memory latency:
/// - Small vectors (≤128D): 4 vectors ahead
/// - Medium vectors (129-512D): 6-8 vectors ahead
/// - Large vectors (513-1024D): 10-12 vectors ahead
/// - Very large vectors (>1024D): 14-16 vectors ahead
#[must_use]
#[inline]
pub fn calculate_prefetch_distance_neon(dimension: usize) -> usize {
    match dimension {
        0..=128 => 4,
        129..=384 => 6,
        385..=768 => 10,
        769..=1536 => 14,
        _ => 16,
    }
}

// Fallback implementations for non-ARM64 targets

/// No-op prefetch for non-ARM64 targets.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
pub fn prefetch_read_l1(_ptr: *const u8) {}

/// No-op prefetch for non-ARM64 targets.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
pub fn prefetch_read_l2(_ptr: *const u8) {}

/// No-op prefetch for non-ARM64 targets.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
pub fn prefetch_read_l3(_ptr: *const u8) {}

/// No-op prefetch for non-ARM64 targets.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
pub fn prefetch_write_l1(_ptr: *const u8) {}

/// No-op prefetch for non-ARM64 targets.
#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
pub fn prefetch_vector_neon(_vector: &[f32]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefetch_read_l1_safe() {
        // Test that prefetch doesn't crash on valid pointer
        let data = vec![0u8; 4096];
        prefetch_read_l1(data.as_ptr());
    }

    #[test]
    fn test_prefetch_read_l1_null_safe() {
        // Test that prefetch doesn't crash on null pointer
        // (prefetch is a hint, should be safe to ignore)
        prefetch_read_l1(std::ptr::null());
    }

    #[test]
    fn test_prefetch_vector_neon() {
        let vector: Vec<f32> = (0..768).map(|i| i as f32).collect();
        prefetch_vector_neon(&vector);
        // No crash = success
    }

    #[test]
    fn test_prefetch_vector_neon_empty() {
        let vector: Vec<f32> = vec![];
        prefetch_vector_neon(&vector);
        // No crash = success
    }

    #[test]
    fn test_calculate_prefetch_distance() {
        assert_eq!(calculate_prefetch_distance_neon(128), 4);
        assert_eq!(calculate_prefetch_distance_neon(384), 6);
        assert_eq!(calculate_prefetch_distance_neon(768), 10);
        assert_eq!(calculate_prefetch_distance_neon(1536), 14);
        assert_eq!(calculate_prefetch_distance_neon(3072), 16);
    }

    #[test]
    fn test_all_prefetch_variants() {
        let data = vec![0u8; 256];
        let ptr = data.as_ptr();

        prefetch_read_l1(ptr);
        prefetch_read_l2(ptr);
        prefetch_read_l3(ptr);
        prefetch_write_l1(ptr);
        // No crash = success
    }
}
