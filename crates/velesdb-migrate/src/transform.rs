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
mod tests {
    use super::*;

    #[test]
    fn test_transform_point_no_mapping() {
        let transformer = Transformer::default();

        let point = ExtractedPoint {
            id: "1".to_string(),
            vector: vec![0.1, 0.2],
            payload: HashMap::from([("title".to_string(), serde_json::json!("Test"))]),
        };

        let result = transformer.transform_point(point);
        assert!(result.payload.contains_key("title"));
    }

    #[test]
    fn test_transform_point_with_mapping() {
        let mappings = HashMap::from([("old_name".to_string(), "new_name".to_string())]);
        let transformer = Transformer::new(mappings);

        let point = ExtractedPoint {
            id: "1".to_string(),
            vector: vec![0.1, 0.2],
            payload: HashMap::from([("old_name".to_string(), serde_json::json!("Test"))]),
        };

        let result = transformer.transform_point(point);
        assert!(result.payload.contains_key("new_name"));
        assert!(!result.payload.contains_key("old_name"));
    }

    #[test]
    fn test_normalize_vector() {
        let vec = vec![3.0, 4.0];
        let normalized = Transformer::normalize_vector(&vec);

        assert!((normalized[0] - 0.6).abs() < 0.001);
        assert!((normalized[1] - 0.8).abs() < 0.001);

        // Check unit length
        let norm: f32 = normalized.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_normalize_zero_vector() {
        let vec = vec![0.0, 0.0, 0.0];
        let normalized = Transformer::normalize_vector(&vec);
        assert_eq!(normalized, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn test_quantize_sq8() {
        let vec = vec![0.0, 0.5, 1.0];
        let quantized = Transformer::quantize_sq8(&vec);

        assert_eq!(quantized[0], 0);
        assert_eq!(quantized[1], 127); // ~128
        assert_eq!(quantized[2], 255);
    }

    #[test]
    fn test_quantize_binary() {
        let vec = vec![1.0, -1.0, 0.5, -0.5, 1.0, -1.0, 0.1, -0.1];
        let binary = Transformer::quantize_binary(&vec);

        // First byte: 1 0 1 0 1 0 1 0 = 0xAA = 170
        assert_eq!(binary.len(), 1);
        assert_eq!(binary[0], 0b10101010);
    }

    #[test]
    fn test_transform_batch() {
        let transformer = Transformer::default();

        let points = vec![
            ExtractedPoint {
                id: "1".to_string(),
                vector: vec![0.1],
                payload: HashMap::new(),
            },
            ExtractedPoint {
                id: "2".to_string(),
                vector: vec![0.2],
                payload: HashMap::new(),
            },
        ];

        let result = transformer.transform_batch(points);
        assert_eq!(result.len(), 2);
    }
}
