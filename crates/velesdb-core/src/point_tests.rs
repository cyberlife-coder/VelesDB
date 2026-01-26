//! Tests for `point` module

use super::point::*;
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

#[test]
fn test_point_metadata_only() {
    let point = Point::metadata_only(42, json!({"category": "test"}));

    assert_eq!(point.id, 42);
    assert!(point.is_metadata_only());
    assert_eq!(point.dimension(), 0);
    assert!(point.payload.is_some());
}

#[test]
fn test_search_result_serialization() {
    let point = Point::new(1, vec![0.1, 0.2], None);
    let result = SearchResult::new(point, 0.85);

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: SearchResult = serde_json::from_str(&json).unwrap();

    assert_eq!(result.point.id, deserialized.point.id);
    assert!((result.score - deserialized.score).abs() < 1e-5);
}
