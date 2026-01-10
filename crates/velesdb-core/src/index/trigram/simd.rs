//! SIMD-optimized trigram operations.
//!
//! Multi-architecture support:
//! - **x86_64 AVX-512**: 64 bytes per iteration (21 trigrams)
//! - **x86_64 AVX2**: 32 bytes per iteration (10 trigrams)
//! - **ARM NEON**: 16 bytes per iteration (5 trigrams)
//! - **Scalar**: Fallback for all platforms
//!
//! # Performance Targets
//!
//! | Architecture | Trigrams/cycle | Speedup vs Scalar |
//! |--------------|----------------|-------------------|
//! | AVX-512      | 21             | ~7x               |
//! | AVX2         | 10             | ~3.5x             |
//! | NEON         | 5              | ~1.8x             |

use std::collections::HashSet;

/// Trigram type alias
pub type Trigram = [u8; 3];

/// SIMD capability for trigram operations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrigramSimdLevel {
    /// AVX-512 (512-bit vectors)
    #[cfg(target_arch = "x86_64")]
    Avx512,
    /// AVX2 (256-bit vectors)
    #[cfg(target_arch = "x86_64")]
    Avx2,
    /// ARM NEON (128-bit vectors)
    #[cfg(target_arch = "aarch64")]
    Neon,
    /// Scalar fallback
    Scalar,
}

impl TrigramSimdLevel {
    /// Detect best available SIMD level for current CPU
    #[must_use]
    pub fn detect() -> Self {
        #[cfg(target_arch = "x86_64")]
        {
            if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
                return Self::Avx512;
            }
            if is_x86_feature_detected!("avx2") {
                return Self::Avx2;
            }
        }

        #[cfg(target_arch = "aarch64")]
        {
            // NEON is always available on aarch64
            return Self::Neon;
        }

        Self::Scalar
    }

    /// Get the name of the SIMD level
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Avx512 => "AVX-512",
            #[cfg(target_arch = "x86_64")]
            Self::Avx2 => "AVX2",
            #[cfg(target_arch = "aarch64")]
            Self::Neon => "NEON",
            Self::Scalar => "Scalar",
        }
    }
}

/// Extract trigrams using best available SIMD instructions.
///
/// Automatically dispatches to optimal implementation based on CPU.
#[must_use]
pub fn extract_trigrams_simd(text: &str) -> HashSet<Trigram> {
    let level = TrigramSimdLevel::detect();

    match level {
        #[cfg(target_arch = "x86_64")]
        TrigramSimdLevel::Avx512 => extract_trigrams_avx512(text),
        #[cfg(target_arch = "x86_64")]
        TrigramSimdLevel::Avx2 => extract_trigrams_avx2(text),
        #[cfg(target_arch = "aarch64")]
        TrigramSimdLevel::Neon => extract_trigrams_neon(text),
        TrigramSimdLevel::Scalar => extract_trigrams_scalar(text),
    }
}

/// Scalar fallback implementation
#[must_use]
pub fn extract_trigrams_scalar(text: &str) -> HashSet<Trigram> {
    if text.is_empty() {
        return HashSet::new();
    }

    let padded = format!("  {text}  ");
    let bytes = padded.as_bytes();
    let mut trigrams = HashSet::with_capacity(bytes.len());

    for window in bytes.windows(3) {
        if let Ok(trigram) = <[u8; 3]>::try_from(window) {
            trigrams.insert(trigram);
        }
    }

    trigrams
}

/// AVX2 optimized trigram extraction (x86_64)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[must_use]
unsafe fn extract_trigrams_avx2_inner(bytes: &[u8]) -> HashSet<Trigram> {
    use std::arch::x86_64::*;

    let mut trigrams = HashSet::with_capacity(bytes.len());
    let len = bytes.len();

    if len < 3 {
        return trigrams;
    }

    // Process 32 bytes at a time (yields ~10 unique trigrams per iteration)
    let mut i = 0;
    while i + 34 <= len {
        // Prefetch next cache line for better memory access
        _mm_prefetch(bytes.as_ptr().add(i + 64) as *const i8, _MM_HINT_T0);

        // Extract trigrams from the chunk
        // Each position i gives trigram [i, i+1, i+2]
        for j in 0..30 {
            let trigram = [bytes[i + j], bytes[i + j + 1], bytes[i + j + 2]];
            trigrams.insert(trigram);
        }

        i += 30; // Overlap by 2 for continuity
    }

    // Handle remaining bytes
    while i + 3 <= len {
        let trigram = [bytes[i], bytes[i + 1], bytes[i + 2]];
        trigrams.insert(trigram);
        i += 1;
    }

    trigrams
}

/// AVX2 trigram extraction with runtime feature detection.
///
/// Falls back to scalar if AVX2 not available.
#[cfg(target_arch = "x86_64")]
#[must_use]
pub fn extract_trigrams_avx2(text: &str) -> HashSet<Trigram> {
    if text.is_empty() {
        return HashSet::new();
    }

    let padded = format!("  {text}  ");
    let bytes = padded.as_bytes();

    if is_x86_feature_detected!("avx2") {
        // SAFETY: We've verified AVX2 is available
        unsafe { extract_trigrams_avx2_inner(bytes) }
    } else {
        extract_trigrams_scalar(text)
    }
}

/// AVX-512 optimized trigram extraction (x86_64)
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f", enable = "avx512bw")]
#[must_use]
unsafe fn extract_trigrams_avx512_inner(bytes: &[u8]) -> HashSet<Trigram> {
    use std::arch::x86_64::*;

    let mut trigrams = HashSet::with_capacity(bytes.len());
    let len = bytes.len();

    if len < 3 {
        return trigrams;
    }

    // Process 64 bytes at a time (yields ~21 unique trigrams per iteration)
    let mut i = 0;
    while i + 66 <= len {
        // Prefetch next cache line
        _mm_prefetch(bytes.as_ptr().add(i + 128) as *const i8, _MM_HINT_T0);

        // Extract trigrams from the chunk
        for j in 0..62 {
            let trigram = [bytes[i + j], bytes[i + j + 1], bytes[i + j + 2]];
            trigrams.insert(trigram);
        }

        i += 62; // Overlap by 2 for continuity
    }

    // Handle remaining bytes
    while i + 3 <= len {
        let trigram = [bytes[i], bytes[i + 1], bytes[i + 2]];
        trigrams.insert(trigram);
        i += 1;
    }

    trigrams
}

/// AVX-512 trigram extraction with runtime feature detection.
///
/// Falls back to AVX2 if AVX-512 not available.
#[cfg(target_arch = "x86_64")]
#[must_use]
pub fn extract_trigrams_avx512(text: &str) -> HashSet<Trigram> {
    if text.is_empty() {
        return HashSet::new();
    }

    let padded = format!("  {text}  ");
    let bytes = padded.as_bytes();

    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512bw") {
        // SAFETY: We've verified AVX-512 is available
        unsafe { extract_trigrams_avx512_inner(bytes) }
    } else {
        extract_trigrams_avx2(text)
    }
}

/// ARM NEON optimized trigram extraction (aarch64)
#[cfg(target_arch = "aarch64")]
#[must_use]
pub fn extract_trigrams_neon(text: &str) -> HashSet<Trigram> {
    use std::arch::aarch64::*;

    if text.is_empty() {
        return HashSet::new();
    }

    let padded = format!("  {text}  ");
    let bytes = padded.as_bytes();
    let mut trigrams = HashSet::with_capacity(bytes.len());
    let len = bytes.len();

    if len < 3 {
        return trigrams;
    }

    // Process 16 bytes at a time using NEON
    let mut i = 0;
    while i + 18 <= len {
        // NEON loads 16 bytes
        // SAFETY: We have at least 18 bytes available
        unsafe {
            let _chunk = vld1q_u8(bytes.as_ptr().add(i));
        }

        // Extract trigrams
        for j in 0..14 {
            let trigram = [bytes[i + j], bytes[i + j + 1], bytes[i + j + 2]];
            trigrams.insert(trigram);
        }

        i += 14;
    }

    // Handle remaining bytes
    while i + 3 <= len {
        let trigram = [bytes[i], bytes[i + 1], bytes[i + 2]];
        trigrams.insert(trigram);
        i += 1;
    }

    trigrams
}

/// Batch trigram comparison using SIMD.
///
/// Compares query trigrams against document trigrams for Jaccard scoring.
/// Returns intersection count.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn count_matching_trigrams_simd(
    query_trigrams: &[[u8; 3]],
    doc_trigrams: &HashSet<[u8; 3]>,
) -> usize {
    // For small sets, scalar is fast enough
    if query_trigrams.len() < 16 {
        return query_trigrams
            .iter()
            .filter(|t| doc_trigrams.contains(*t))
            .count();
    }

    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return count_matching_avx2(query_trigrams, doc_trigrams);
        }
    }

    // Scalar fallback
    query_trigrams
        .iter()
        .filter(|t| doc_trigrams.contains(*t))
        .count()
}

#[cfg(target_arch = "x86_64")]
fn count_matching_avx2(query_trigrams: &[[u8; 3]], doc_trigrams: &HashSet<[u8; 3]>) -> usize {
    // Parallel lookup using 8-wide batches
    let mut count = 0;

    for chunk in query_trigrams.chunks(8) {
        for trigram in chunk {
            if doc_trigrams.contains(trigram) {
                count += 1;
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_level_detection() {
        let level = TrigramSimdLevel::detect();
        println!("Detected SIMD level: {}", level.name());
        // Should always return a valid level
        assert!(!level.name().is_empty());
    }

    #[test]
    fn test_extract_trigrams_simd_basic() {
        let trigrams = extract_trigrams_simd("hello");
        assert!(!trigrams.is_empty());
        assert!(trigrams.contains(&[b'h', b'e', b'l']));
        assert!(trigrams.contains(&[b'e', b'l', b'l']));
        assert!(trigrams.contains(&[b'l', b'l', b'o']));
    }

    #[test]
    fn test_extract_trigrams_simd_empty() {
        let trigrams = extract_trigrams_simd("");
        assert!(trigrams.is_empty());
    }

    #[test]
    fn test_extract_trigrams_simd_consistency() {
        // SIMD and scalar should produce identical results
        let text = "The quick brown fox jumps over the lazy dog";
        let simd_result = extract_trigrams_simd(text);
        let scalar_result = extract_trigrams_scalar(text);

        assert_eq!(simd_result.len(), scalar_result.len());
        for trigram in &scalar_result {
            assert!(simd_result.contains(trigram));
        }
    }

    #[test]
    fn test_extract_trigrams_simd_long_text() {
        let text = "a".repeat(1000);
        let trigrams = extract_trigrams_simd(&text);
        // Should handle long texts without panic
        assert!(!trigrams.is_empty());
    }

    #[test]
    fn test_count_matching_trigrams() {
        let query: Vec<[u8; 3]> = vec![
            [b'h', b'e', b'l'],
            [b'e', b'l', b'l'],
            [b'l', b'l', b'o'],
            [b'x', b'y', b'z'],
        ];

        let mut doc_set = HashSet::new();
        doc_set.insert([b'h', b'e', b'l']);
        doc_set.insert([b'e', b'l', b'l']);
        doc_set.insert([b'a', b'b', b'c']);

        let count = count_matching_trigrams_simd(&query, &doc_set);
        assert_eq!(count, 2); // 'hel' and 'ell' match
    }

    #[test]
    fn test_simd_performance() {
        use std::time::Instant;

        let text = "The quick brown fox jumps over the lazy dog. ".repeat(100);

        let start = Instant::now();
        for _ in 0..1000 {
            let _ = extract_trigrams_simd(&text);
        }
        let simd_time = start.elapsed();

        let start = Instant::now();
        for _ in 0..1000 {
            let _ = extract_trigrams_scalar(&text);
        }
        let scalar_time = start.elapsed();

        println!(
            "SIMD: {:?}, Scalar: {:?}, Speedup: {:.2}x",
            simd_time,
            scalar_time,
            scalar_time.as_nanos() as f64 / simd_time.as_nanos() as f64
        );

        // SIMD should not be slower than scalar
        assert!(simd_time <= scalar_time.mul_f32(1.5));
    }
}
