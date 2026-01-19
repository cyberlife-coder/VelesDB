//! Vector to bytes conversion utilities for storage.
//!
//! Provides safe conversion between `&[f32]` vectors and `&[u8]` byte slices
//! for persistence in memory-mapped storage.

/// Converts a vector slice to a byte slice.
///
/// # Safety
///
/// This is safe because:
/// - f32 has no invalid bit patterns
/// - The slice layout is well-defined
/// - The lifetime of the returned slice is tied to the input
#[inline]
pub(super) fn vector_to_bytes(vector: &[f32]) -> &[u8] {
    // SAFETY: f32 has no invalid bit patterns, slice is contiguous, lifetime preserved
    unsafe {
        std::slice::from_raw_parts(vector.as_ptr().cast::<u8>(), std::mem::size_of_val(vector))
    }
}

/// Converts bytes back to a vector.
///
/// # Arguments
///
/// * `bytes` - Raw bytes to convert (must be at least `dimension * 4` bytes)
/// * `dimension` - Expected vector dimension
///
/// # Returns
///
/// A new `Vec<f32>` containing the converted data.
///
/// # Panics
///
/// Panics if `bytes.len() < dimension * size_of::<f32>()`.
#[inline]
pub(super) fn bytes_to_vector(bytes: &[u8], dimension: usize) -> Vec<f32> {
    let vector_size = dimension * std::mem::size_of::<f32>();
    assert!(
        bytes.len() >= vector_size,
        "bytes_to_vector: buffer too small ({} < {})",
        bytes.len(),
        vector_size
    );

    let mut vector = vec![0.0f32; dimension];
    // SAFETY: We've verified bytes.len() >= vector_size above
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            vector.as_mut_ptr().cast::<u8>(),
            vector_size,
        );
    }
    vector
}
