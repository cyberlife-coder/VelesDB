//! Trigram Index for LIKE/ILIKE acceleration.
//!
//! This module implements a trigram-based inverted index using Roaring Bitmaps
//! for efficient pattern matching queries (LIKE '%pattern%').
//!
//! # Architecture (SOTA 2026)
//!
//! Based on arXiv:2310.11703v2 and `PostgreSQL` `pg_trgm`:
//! - Extract trigrams from text with padding
//! - Store inverted index: trigram → `RoaringBitmap` of doc IDs
//! - Query: intersect bitmaps for all query trigrams
//! - Scoring: Jaccard similarity for ranking
//!
//! # Multi-Architecture SIMD Support
//!
//! | Architecture | Instruction Set | Trigrams/cycle |
//! |--------------|-----------------|----------------|
//! | x86_64       | AVX-512         | ~21            |
//! | x86_64       | AVX2            | ~10            |
//! | aarch64      | NEON            | ~5             |
//! | Fallback     | Scalar          | ~3             |
//!
//! # Performance Targets
//!
//! | Volume | Without Index | With Trigram | Speedup |
//! |--------|---------------|--------------|---------|
//! | 10K    | 45ms          | < 5ms        | > 9x    |
//! | 100K   | 450ms         | < 20ms       | > 22x   |
//! | 1M     | 4.5s          | < 100ms      | > 45x   |

pub mod gpu;
mod index;
pub mod simd;

pub use gpu::TrigramComputeBackend;
pub use index::{extract_trigrams, TrigramIndex};
pub use simd::{extract_trigrams_simd, TrigramSimdLevel};

#[cfg(test)]
mod simd_tests;
#[cfg(test)]
mod thread_safety_tests;

#[cfg(test)]
pub use thread_safety_tests::ConcurrentTrigramIndex;

#[cfg(test)]
mod tests;
