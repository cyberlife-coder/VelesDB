//! Tests for Collection module.

use super::*;
use crate::distance::DistanceMetric;
use crate::point::Point;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn test_collection_create() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    let config = collection.config();

    assert_eq!(config.dimension, 3);
    assert_eq!(config.metric, DistanceMetric::Cosine);
    assert_eq!(config.point_count, 0);
}

#[test]
fn test_collection_upsert_and_search() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::without_payload(1, vec![1.0, 0.0, 0.0]),
        Point::without_payload(2, vec![0.0, 1.0, 0.0]),
        Point::without_payload(3, vec![0.0, 0.0, 1.0]),
    ];

    collection.upsert(points).unwrap();
    assert_eq!(collection.len(), 3);

    let query = vec![1.0, 0.0, 0.0];
    let results = collection.search(&query, 2).unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].point.id, 1); // Most similar
}

#[test]
fn test_dimension_mismatch() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![Point::without_payload(1, vec![1.0, 0.0])]; // Wrong dimension

    let result = collection.upsert(points);
    assert!(result.is_err());
}

#[test]
fn test_collection_open_existing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    // Create and populate collection
    {
        let collection = Collection::create(path.clone(), 3, DistanceMetric::Euclidean).unwrap();
        let points = vec![
            Point::without_payload(1, vec![1.0, 2.0, 3.0]),
            Point::without_payload(2, vec![4.0, 5.0, 6.0]),
        ];
        collection.upsert(points).unwrap();
        collection.flush().unwrap();
    }

    // Reopen and verify
    let collection = Collection::open(path).unwrap();
    let config = collection.config();

    assert_eq!(config.dimension, 3);
    assert_eq!(config.metric, DistanceMetric::Euclidean);
    assert_eq!(collection.len(), 2);
}

#[test]
fn test_collection_get_points() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    let points = vec![
        Point::without_payload(1, vec![1.0, 0.0, 0.0]),
        Point::without_payload(2, vec![0.0, 1.0, 0.0]),
    ];
    collection.upsert(points).unwrap();

    // Get existing points
    let retrieved = collection.get(&[1, 2, 999]);

    assert!(retrieved[0].is_some());
    assert_eq!(retrieved[0].as_ref().unwrap().id, 1);
    assert!(retrieved[1].is_some());
    assert_eq!(retrieved[1].as_ref().unwrap().id, 2);
    assert!(retrieved[2].is_none()); // 999 doesn't exist
}

#[test]
fn test_collection_delete_points() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    let points = vec![
        Point::without_payload(1, vec![1.0, 0.0, 0.0]),
        Point::without_payload(2, vec![0.0, 1.0, 0.0]),
        Point::without_payload(3, vec![0.0, 0.0, 1.0]),
    ];
    collection.upsert(points).unwrap();
    assert_eq!(collection.len(), 3);

    // Delete one point
    collection.delete(&[2]).unwrap();
    assert_eq!(collection.len(), 2);

    // Verify it's gone
    let retrieved = collection.get(&[2]);
    assert!(retrieved[0].is_none());
}

#[test]
fn test_collection_is_empty() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    assert!(collection.is_empty());

    collection
        .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
        .unwrap();
    assert!(!collection.is_empty());
}

#[test]
fn test_collection_with_payload() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0],
        Some(json!({"title": "Test Document", "category": "tech"})),
    )];
    collection.upsert(points).unwrap();

    let retrieved = collection.get(&[1]);
    assert!(retrieved[0].is_some());

    let point = retrieved[0].as_ref().unwrap();
    assert!(point.payload.is_some());
    assert_eq!(point.payload.as_ref().unwrap()["title"], "Test Document");
}

#[test]
fn test_collection_search_dimension_mismatch() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    collection
        .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
        .unwrap();

    // Search with wrong dimension
    let result = collection.search(&[1.0, 0.0], 5);
    assert!(result.is_err());
}

#[test]
fn test_collection_search_ids_fast() {
    // Round 8: Test fast search returning only IDs and scores
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    collection
        .upsert(vec![
            Point::without_payload(1, vec![1.0, 0.0, 0.0]),
            Point::without_payload(2, vec![0.9, 0.1, 0.0]),
            Point::without_payload(3, vec![0.0, 1.0, 0.0]),
        ])
        .unwrap();

    // Fast search returns (id, score) tuples
    let results = collection.search_ids(&[1.0, 0.0, 0.0], 2).unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, 1); // Best match
    assert!(results[0].1 > results[1].1); // Scores are sorted
}

#[test]
fn test_collection_upsert_replaces_payload() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    // Insert with payload
    collection
        .upsert(vec![Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"version": 1})),
        )])
        .unwrap();

    // Upsert without payload (should clear it)
    collection
        .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
        .unwrap();

    let retrieved = collection.get(&[1]);
    let point = retrieved[0].as_ref().unwrap();
    assert!(point.payload.is_none());
}

#[test]
fn test_collection_flush() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();
    collection
        .upsert(vec![Point::without_payload(1, vec![1.0, 0.0, 0.0])])
        .unwrap();

    // Explicit flush should succeed
    let result = collection.flush();
    assert!(result.is_ok());
}

#[test]
fn test_collection_euclidean_metric() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Euclidean).unwrap();

    let points = vec![
        Point::without_payload(1, vec![0.0, 0.0, 0.0]),
        Point::without_payload(2, vec![1.0, 0.0, 0.0]),
        Point::without_payload(3, vec![10.0, 0.0, 0.0]),
    ];
    collection.upsert(points).unwrap();

    let query = vec![0.5, 0.0, 0.0];
    let results = collection.search(&query, 3).unwrap();

    // Point 1 (0,0,0) and Point 2 (1,0,0) should be closest to query (0.5,0,0)
    assert!(results[0].point.id == 1 || results[0].point.id == 2);
}

#[test]
fn test_collection_text_search() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"title": "Rust Programming", "content": "Learn Rust language"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0, 0.0],
            Some(json!({"title": "Python Tutorial", "content": "Python is great"})),
        ),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0],
            Some(json!({"title": "Rust Performance", "content": "Rust is fast"})),
        ),
    ];
    collection.upsert(points).unwrap();

    // Search for "rust" - should match docs 1 and 3
    let results = collection.text_search("rust", 10);
    assert_eq!(results.len(), 2);

    let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
    assert!(ids.contains(&1));
    assert!(ids.contains(&3));
}

#[test]
fn test_collection_hybrid_search() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"title": "Rust Programming"})),
        ),
        Point::new(
            2,
            vec![0.9, 0.1, 0.0], // Similar vector to query
            Some(json!({"title": "Python Programming"})),
        ),
        Point::new(
            3,
            vec![0.0, 1.0, 0.0],
            Some(json!({"title": "Rust Performance"})),
        ),
    ];
    collection.upsert(points).unwrap();

    // Hybrid search: vector close to [1,0,0], text "rust"
    // Doc 1 matches both (vector + text)
    // Doc 2 matches vector only
    // Doc 3 matches text only
    let query = vec![1.0, 0.0, 0.0];
    let results = collection
        .hybrid_search(&query, "rust", 3, Some(0.5))
        .unwrap();

    assert!(!results.is_empty());
    // Doc 1 should rank high (matches both)
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_extract_text_from_payload() {
    // Test nested payload extraction
    let payload = json!({
        "title": "Hello",
        "meta": {
            "author": "World",
            "tags": ["rust", "fast"]
        }
    });

    let text = Collection::extract_text_from_payload(&payload);
    assert!(text.contains("Hello"));
    assert!(text.contains("World"));
    assert!(text.contains("rust"));
    assert!(text.contains("fast"));
}

#[test]
fn test_text_search_empty_query() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0],
        Some(json!({"content": "test document"})),
    )];
    collection.upsert(points).unwrap();

    // Empty query should return empty results
    let results = collection.text_search("", 10);
    assert!(results.is_empty());
}

#[test]
fn test_text_search_no_payload() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    // Points without payload
    let points = vec![
        Point::new(1, vec![1.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0], None),
    ];
    collection.upsert(points).unwrap();

    // Text search should return empty (no text indexed)
    let results = collection.text_search("test", 10);
    assert!(results.is_empty());
}

#[test]
fn test_hybrid_search_text_weight_zero() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(1, vec![1.0, 0.0, 0.0], Some(json!({"title": "Rust"}))),
        Point::new(2, vec![0.9, 0.1, 0.0], Some(json!({"title": "Python"}))),
    ];
    collection.upsert(points).unwrap();

    // vector_weight=1.0 means text_weight=0.0 (pure vector search)
    let query = vec![0.9, 0.1, 0.0];
    let results = collection
        .hybrid_search(&query, "rust", 2, Some(1.0))
        .unwrap();

    // Doc 2 should be first (closest vector) even though "rust" matches doc 1
    assert_eq!(results[0].point.id, 2);
}

#[test]
fn test_hybrid_search_vector_weight_zero() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"title": "Rust programming language"})),
        ),
        Point::new(
            2,
            vec![0.99, 0.01, 0.0], // Very close to query vector
            Some(json!({"title": "Python programming"})),
        ),
    ];
    collection.upsert(points).unwrap();

    // vector_weight=0.0 means text_weight=1.0 (pure text search)
    let query = vec![0.99, 0.01, 0.0];
    let results = collection
        .hybrid_search(&query, "rust", 2, Some(0.0))
        .unwrap();

    // Doc 1 should be first (matches "rust") even though doc 2 has closer vector
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_bm25_update_document() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    // Insert initial document
    let points = vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0],
        Some(json!({"content": "rust programming"})),
    )];
    collection.upsert(points).unwrap();

    // Verify it's indexed
    let results = collection.text_search("rust", 10);
    assert_eq!(results.len(), 1);

    // Update document with different text
    let points = vec![Point::new(
        1,
        vec![1.0, 0.0, 0.0],
        Some(json!({"content": "python programming"})),
    )];
    collection.upsert(points).unwrap();

    // Should no longer match "rust"
    let results = collection.text_search("rust", 10);
    assert!(results.is_empty());

    // Should now match "python"
    let results = collection.text_search("python", 10);
    assert_eq!(results.len(), 1);
}

#[test]
fn test_bm25_large_dataset() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    let collection = Collection::create(path, 4, DistanceMetric::Cosine).unwrap();

    // Insert 100 documents
    let points: Vec<Point> = (0..100)
        .map(|i| {
            let content = if i % 10 == 0 {
                format!("rust document number {i}")
            } else {
                format!("other document number {i}")
            };
            Point::new(
                i,
                vec![0.1, 0.2, 0.3, 0.4],
                Some(json!({"content": content})),
            )
        })
        .collect();
    collection.upsert(points).unwrap();

    // Search for "rust" - should find 10 documents (0, 10, 20, ..., 90)
    let results = collection.text_search("rust", 100);
    assert_eq!(results.len(), 10);

    // All results should have IDs divisible by 10
    for result in &results {
        assert_eq!(result.point.id % 10, 0);
    }
}

#[test]
fn test_bm25_persistence_on_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");

    // Create collection and add documents
    {
        let collection = Collection::create(path.clone(), 4, DistanceMetric::Cosine).unwrap();

        let points = vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"content": "Rust programming language"})),
            ),
            Point::new(
                2,
                vec![0.0, 1.0, 0.0, 0.0],
                Some(json!({"content": "Python tutorial"})),
            ),
            Point::new(
                3,
                vec![0.0, 0.0, 1.0, 0.0],
                Some(json!({"content": "Rust is fast and safe"})),
            ),
        ];
        collection.upsert(points).unwrap();

        // Verify search works before closing
        let results = collection.text_search("rust", 10);
        assert_eq!(results.len(), 2);
    }

    // Reopen collection and verify BM25 index is rebuilt
    {
        let collection = Collection::open(path).unwrap();

        // BM25 should be rebuilt from persisted payloads
        let results = collection.text_search("rust", 10);
        assert_eq!(results.len(), 2);

        let ids: Vec<u64> = results.iter().map(|r| r.point.id).collect();
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
    }
}

// =========================================================================
// Tests for upsert_bulk (optimized bulk import)
// =========================================================================

#[test]
fn test_upsert_bulk_basic() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(1, vec![1.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0], None),
    ];

    let inserted = collection.upsert_bulk(&points).unwrap();
    assert_eq!(inserted, 3);
    assert_eq!(collection.len(), 3);
}

#[test]
fn test_upsert_bulk_with_payload() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(1, vec![1.0, 0.0, 0.0], Some(json!({"title": "Doc 1"}))),
        Point::new(2, vec![0.0, 1.0, 0.0], Some(json!({"title": "Doc 2"}))),
    ];

    collection.upsert_bulk(&points).unwrap();
    let retrieved = collection.get(&[1, 2]);
    assert_eq!(retrieved.len(), 2);
    assert!(retrieved[0].as_ref().unwrap().payload.is_some());
}

#[test]
fn test_upsert_bulk_empty() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points: Vec<Point> = vec![];
    let inserted = collection.upsert_bulk(&points).unwrap();
    assert_eq!(inserted, 0);
}

#[test]
fn test_upsert_bulk_dimension_mismatch() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(1, vec![1.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0], None), // Wrong dimension
    ];

    let result = collection.upsert_bulk(&points);
    assert!(result.is_err());
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn test_upsert_bulk_large_batch() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 64, DistanceMetric::Cosine).unwrap();

    let points: Vec<Point> = (0_u64..500)
        .map(|i| {
            let vector: Vec<f32> = (0_u64..64)
                .map(|j| ((i + j) % 100) as f32 / 100.0)
                .collect();
            Point::new(i, vector, None)
        })
        .collect();

    let inserted = collection.upsert_bulk(&points).unwrap();
    assert_eq!(inserted, 500);
    assert_eq!(collection.len(), 500);
}

#[test]
fn test_upsert_bulk_search_works() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    // Use more distinct vectors to ensure deterministic search results
    let points = vec![
        Point::new(1, vec![1.0, 0.0, 0.0], None),
        Point::new(2, vec![0.0, 1.0, 0.0], None),
        Point::new(3, vec![0.0, 0.0, 1.0], None),
    ];

    collection.upsert_bulk(&points).unwrap();

    let query = vec![1.0, 0.0, 0.0];
    let results = collection.search(&query, 3).unwrap();
    assert!(!results.is_empty());
    // With distinct orthogonal vectors, id=1 should always be the top result
    assert_eq!(results[0].point.id, 1);
}

#[test]
fn test_upsert_bulk_bm25_indexing() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_collection");
    let collection = Collection::create(path, 3, DistanceMetric::Cosine).unwrap();

    let points = vec![
        Point::new(
            1,
            vec![1.0, 0.0, 0.0],
            Some(json!({"content": "Rust lang"})),
        ),
        Point::new(2, vec![0.0, 1.0, 0.0], Some(json!({"content": "Python"}))),
        Point::new(
            3,
            vec![0.0, 0.0, 1.0],
            Some(json!({"content": "Rust fast"})),
        ),
    ];

    collection.upsert_bulk(&points).unwrap();
    let results = collection.text_search("rust", 10);
    assert_eq!(results.len(), 2);
}
