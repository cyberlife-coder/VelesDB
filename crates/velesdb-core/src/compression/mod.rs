//! Column compression for `VelesDB` (SOTA 2026).
//!
//! Based on arXiv:2310.11703v2 recommendations:
//! - Dictionary encoding for repeated values
//! - Delta encoding for sequential numbers
//! - Run-length encoding for consecutive duplicates

mod dictionary;

pub use dictionary::{CompressionStats, DictCodebook, DictionaryEncoder};

#[cfg(test)]
mod tests;
