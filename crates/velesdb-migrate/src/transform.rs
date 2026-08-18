//! Data transformation utilities.

use std::collections::HashMap;

use crate::connectors::ExtractedPoint;

/// Transforms extracted data before loading.
pub struct Transformer {
    /// Field mappings (source -> dest).
    field_mappings: HashMap<String, String>,
}

impl Transformer {
    /// Create a new transformer.
    #[must_use]
    pub fn new(field_mappings: HashMap<String, String>) -> Self {
        Self { field_mappings }
    }

    /// Transform a batch of points.
    #[must_use]
    pub fn transform_batch(&self, points: Vec<ExtractedPoint>) -> Vec<ExtractedPoint> {
        points
            .into_iter()
            .map(|p| self.transform_point(p))
            .collect()
    }

    /// Transform a single point.
    #[must_use]
    pub fn transform_point(&self, mut point: ExtractedPoint) -> ExtractedPoint {
        if !self.field_mappings.is_empty() {
            let mut new_payload = HashMap::new();

            for (key, value) in point.payload.drain() {
                let new_key = self.field_mappings.get(&key).cloned().unwrap_or(key);
                new_payload.insert(new_key, value);
            }

            point.payload = new_payload;
        }

        point
    }

    /// Normalize a vector to unit length.
    #[must_use]
    pub fn normalize_vector(vector: &[f32]) -> Vec<f32> {
        let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            vector.iter().map(|x| x / norm).collect()
        } else {
            vector.to_vec()
        }
    }

    /// Quantize vector to SQ8 (scalar quantization).
    #[must_use]
    pub fn quantize_sq8(vector: &[f32]) -> Vec<u8> {
        let min = vector.iter().copied().fold(f32::INFINITY, f32::min);
        let max = vector.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let range = max - min;

        if range == 0.0 {
            return vec![128u8; vector.len()];
        }

        vector
            .iter()
            .map(|&x| ((x - min) / range * 255.0) as u8)
            .collect()
    }

    /// Quantize vector to binary (1-bit).
    #[must_use]
    pub fn quantize_binary(vector: &[f32]) -> Vec<u8> {
        let bytes_needed = vector.len().div_ceil(8);
        let mut result = vec![0u8; bytes_needed];

        for (i, &val) in vector.iter().enumerate() {
            if val > 0.0 {
                result[i / 8] |= 1 << (7 - (i % 8));
            }
        }

        result
    }
}

impl Default for Transformer {
    fn default() -> Self {
        Self::new(HashMap::new())
    }
}

#[cfg(test)]
#[path = "transform_tests.rs"]
mod tests;
