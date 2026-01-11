//! Dictionary Encoding for column compression.
//!
//! Replaces repeated values with compact integer codes.
//! Ideal for columns with low cardinality (e.g., country, category).

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_precision_loss)]

use rustc_hash::FxHashMap;
use std::hash::Hash;
use std::mem::size_of;

/// Compression statistics.
#[derive(Debug, Clone, Default)]
pub struct CompressionStats {
    /// Number of unique values in dictionary.
    pub unique_values: usize,
    /// Total number of encoded values.
    pub total_values: usize,
    /// Dictionary size in bytes.
    pub dictionary_size_bytes: usize,
    /// Encoded data size in bytes.
    pub encoded_size_bytes: usize,
    /// Compression ratio (original / compressed).
    pub compression_ratio: f64,
}

/// Dictionary codebook mapping values to codes.
#[derive(Debug, Clone)]
pub struct DictCodebook<V> {
    /// Value to code mapping.
    value_to_code: FxHashMap<V, u32>,
    /// Code to value mapping.
    code_to_value: Vec<V>,
}

impl<V: Hash + Eq + Clone> Default for DictCodebook<V> {
    fn default() -> Self {
        Self {
            value_to_code: FxHashMap::default(),
            code_to_value: Vec::new(),
        }
    }
}

/// Dictionary encoder for column compression.
///
/// Encodes values as compact integer codes using a codebook.
#[derive(Debug, Clone)]
pub struct DictionaryEncoder<V: Hash + Eq + Clone> {
    /// The codebook.
    codebook: DictCodebook<V>,
    /// Number of values encoded (including duplicates).
    total_encoded: usize,
}

impl<V: Hash + Eq + Clone> DictionaryEncoder<V> {
    /// Create a new dictionary encoder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            codebook: DictCodebook::default(),
            total_encoded: 0,
        }
    }

    /// Check if the dictionary is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.codebook.code_to_value.is_empty()
    }

    /// Get the number of unique values in the dictionary.
    #[must_use]
    pub fn len(&self) -> usize {
        self.codebook.code_to_value.len()
    }

    /// Encode a single value, returning its code.
    ///
    /// If the value is new, it's added to the dictionary.
    pub fn encode(&mut self, value: V) -> u32 {
        self.total_encoded += 1;

        if let Some(&code) = self.codebook.value_to_code.get(&value) {
            return code;
        }

        let code = self.codebook.code_to_value.len() as u32;
        self.codebook.value_to_code.insert(value.clone(), code);
        self.codebook.code_to_value.push(value);
        code
    }

    /// Decode a code back to its value.
    #[must_use]
    pub fn decode(&self, code: u32) -> Option<&V> {
        self.codebook.code_to_value.get(code as usize)
    }

    /// Encode a batch of values.
    pub fn encode_batch(&mut self, values: &[V]) -> Vec<u32> {
        values.iter().map(|v| self.encode(v.clone())).collect()
    }

    /// Decode a batch of codes.
    #[must_use]
    pub fn decode_batch(&self, codes: &[u32]) -> Vec<V> {
        codes
            .iter()
            .filter_map(|&code| self.decode(code).cloned())
            .collect()
    }

    /// Clear the encoder.
    pub fn clear(&mut self) {
        self.codebook.value_to_code.clear();
        self.codebook.code_to_value.clear();
        self.total_encoded = 0;
    }

    /// Get compression statistics.
    #[must_use]
    pub fn stats(&self) -> CompressionStats {
        let unique = self.len();
        let total = self.total_encoded;

        // Estimate sizes
        let value_size = size_of::<V>();
        let original_size = total * value_size;
        let dict_size = unique * value_size + unique * 4; // value + code
        let encoded_size = total * 4; // u32 codes
        let compressed_size = dict_size + encoded_size;

        let ratio = if compressed_size > 0 {
            original_size as f64 / compressed_size as f64
        } else {
            0.0
        };

        CompressionStats {
            unique_values: unique,
            total_values: total,
            dictionary_size_bytes: dict_size,
            encoded_size_bytes: encoded_size,
            compression_ratio: ratio,
        }
    }

    /// Get the codebook.
    #[must_use]
    pub fn codebook(&self) -> &DictCodebook<V> {
        &self.codebook
    }
}

impl<V: Hash + Eq + Clone> Default for DictionaryEncoder<V> {
    fn default() -> Self {
        Self::new()
    }
}
