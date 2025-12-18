//! Point data structure representing a vector with metadata.

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

/// A point in the vector database.
///
/// A point consists of:
/// - A unique identifier
/// - A vector (embedding)
/// - Optional payload (metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    /// Unique identifier for the point.
    pub id: u64,

    /// The vector embedding.
    pub vector: Vec<f32>,

    /// Optional JSON payload containing metadata.
    #[serde(default)]
    pub payload: Option<JsonValue>,
}

impl Point {
    /// Creates a new point with the given ID, vector, and optional payload.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier
    /// * `vector` - Vector embedding
    /// * `payload` - Optional metadata
    #[must_use]
    pub fn new(id: u64, vector: Vec<f32>, payload: Option<JsonValue>) -> Self {
        Self {
            id,
            vector,
            payload,
        }
    }

    /// Creates a new point without payload.
    #[must_use]
    pub fn without_payload(id: u64, vector: Vec<f32>) -> Self {
        Self::new(id, vector, None)
    }

    /// Returns the dimension of the vector.
    #[must_use]
    pub fn dimension(&self) -> usize {
        self.vector.len()
    }
}

/// A search result containing a point and its similarity score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matching point.
    pub point: Point,

    /// Similarity score (interpretation depends on the distance metric).
    pub score: f32,
}

impl SearchResult {
    /// Creates a new search result.
    #[must_use]
    pub const fn new(point: Point, score: f32) -> Self {
        Self { point, score }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_point_creation() {
        let point = Point::new(1, vec![0.1, 0.2, 0.3], Some(json!({"title": "Test"})));

        assert_eq!(point.id, 1);
        assert_eq!(point.dimension(), 3);
        assert!(point.payload.is_some());
    }

    #[test]
    fn test_point_without_payload() {
        let point = Point::without_payload(1, vec![0.1, 0.2, 0.3]);

        assert_eq!(point.id, 1);
        assert!(point.payload.is_none());
    }

    #[test]
    fn test_point_serialization() {
        let point = Point::new(1, vec![0.1, 0.2], Some(json!({"key": "value"})));
        let json = serde_json::to_string(&point).unwrap();
        let deserialized: Point = serde_json::from_str(&json).unwrap();

        assert_eq!(point.id, deserialized.id);
        assert_eq!(point.vector, deserialized.vector);
    }
}
