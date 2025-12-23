//! Integration tests for `VelesDB` real-world usage scenarios.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
    clippy::uninlined_format_args
)]
//!
//! These tests simulate complete workflows that users would perform
//! in production environments.

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, Point};

/// Helper to create a collection and get it back
fn create_and_get_collection(
    db: &Database,
    name: &str,
    dimension: usize,
    metric: DistanceMetric,
) -> velesdb_core::Collection {
    db.create_collection(name, dimension, metric)
        .expect("Failed to create collection");
    db.get_collection(name).expect("Failed to get collection")
}

// =============================================================================
// SCENARIO 1: RAG (Retrieval-Augmented Generation) Pipeline
// =============================================================================

mod rag_pipeline {
    use super::*;

    fn create_mock_embedding(seed: u64, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin())
            .collect()
    }

    #[test]
    fn test_rag_complete_workflow() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection =
            create_and_get_collection(&db, "knowledge_base", 384, DistanceMetric::Cosine);

        // Ingest document chunks with embeddings and metadata
        let documents = [
            ("Introduction to Rust programming", "rust"),
            ("Memory safety in Rust", "rust"),
            ("Python for data science", "python"),
            ("Machine learning basics", "ml"),
            ("Vector databases explained", "database"),
            ("HNSW algorithm deep dive", "database"),
            ("Rust async programming", "rust"),
            ("Building REST APIs with Axum", "rust"),
        ];

        let points: Vec<Point> = documents
            .iter()
            .enumerate()
            .map(|(id, (content, category))| {
                let embedding = create_mock_embedding(id as u64, 384);
                let payload = json!({
                    "content": content,
                    "category": category
                });
                Point::new(id as u64, embedding, Some(payload))
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert points");

        // Verify document count
        assert_eq!(collection.len(), 8, "Should have 8 documents");

        // Semantic search for content similar to first doc
        let query_embedding = create_mock_embedding(0, 384);
        let results = collection
            .search(&query_embedding, 5)
            .expect("Search failed");

        assert!(!results.is_empty(), "Should find related documents");
        assert!(results.len() <= 5, "Should respect top_k limit");

        // First result should be the exact match
        assert_eq!(results[0].point.id, 0, "First result should be exact match");
    }

    #[test]
    fn test_rag_incremental_updates() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "docs", 128, DistanceMetric::Cosine);

        // Initial batch
        let initial_points: Vec<Point> = (0..5)
            .map(|id| Point::without_payload(id, create_mock_embedding(id, 128)))
            .collect();
        collection.upsert(initial_points).expect("Failed to upsert");
        assert_eq!(collection.len(), 5);

        // Incremental update
        let more_points: Vec<Point> = (5..10)
            .map(|id| Point::without_payload(id, create_mock_embedding(id, 128)))
            .collect();
        collection.upsert(more_points).expect("Failed to upsert");
        assert_eq!(collection.len(), 10);

        // Update existing document (upsert with same ID)
        let updated_point = Point::without_payload(0, create_mock_embedding(100, 128));
        collection
            .upsert(vec![updated_point])
            .expect("Failed to update");
        assert_eq!(
            collection.len(),
            10,
            "Count should remain the same after update"
        );
    }

    #[test]
    fn test_rag_delete_and_search() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "docs", 128, DistanceMetric::Cosine);

        // Add documents
        let points: Vec<Point> = (0..10)
            .map(|id| Point::without_payload(id, create_mock_embedding(id, 128)))
            .collect();
        collection.upsert(points).expect("Failed to upsert");

        // Delete some documents
        collection.delete(&[5, 7]).expect("Failed to delete");
        assert_eq!(
            collection.len(),
            8,
            "Should have 8 documents after deletion"
        );

        // Search should still work
        let query = create_mock_embedding(0, 128);
        let results = collection.search(&query, 5).expect("Search failed");
        assert!(!results.is_empty());

        // Deleted IDs should not appear in results
        for result in &results {
            assert!(
                result.point.id != 5 && result.point.id != 7,
                "Deleted IDs should not appear"
            );
        }
    }
}

// =============================================================================
// SCENARIO 2: E-commerce Product Search
// =============================================================================
// Simulates semantic product search:
// 1. Index products with embeddings + metadata
// 2. Search by embedding similarity

mod ecommerce_search {
    use super::*;

    fn product_embedding(product_id: u64) -> Vec<f32> {
        (0..768)
            .map(|i| (product_id as f32 * 0.1 + i as f32 * 0.001).cos())
            .collect()
    }

    #[test]
    fn test_product_catalog_indexing() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "products", 768, DistanceMetric::Cosine);

        // Index products with rich metadata
        let products = [
            (1u64, "Wireless Bluetooth Headphones", "electronics", 79.99),
            (2, "USB-C Charging Cable", "electronics", 12.99),
            (3, "Mechanical Keyboard RGB", "electronics", 149.99),
            (4, "Running Shoes Pro", "sports", 129.99),
            (5, "Yoga Mat Premium", "sports", 39.99),
            (6, "Coffee Maker Deluxe", "home", 89.99),
            (7, "Smart Watch Series X", "electronics", 299.99),
            (8, "Fitness Tracker Band", "electronics", 49.99),
        ];

        let points: Vec<Point> = products
            .iter()
            .map(|(id, name, category, price)| {
                let payload = json!({
                    "name": name,
                    "category": category,
                    "price": price
                });
                Point::new(*id, product_embedding(*id), Some(payload))
            })
            .collect();

        collection.upsert(points).expect("Failed to index products");
        assert_eq!(collection.len(), 8);

        // Search for products similar to first one
        let query = product_embedding(1);
        let results = collection.search(&query, 10).expect("Search failed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1, "First result should be exact match");
    }

    #[test]
    fn test_batch_product_indexing_performance() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "products", 128, DistanceMetric::Cosine);

        // Batch insert 1000 products
        let points: Vec<Point> = (0..1000)
            .map(|id| {
                let embedding: Vec<f32> = (0..128)
                    .map(|i| (id as f32 * 0.1 + i as f32 * 0.01).cos())
                    .collect();
                Point::without_payload(id, embedding)
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");
        assert_eq!(collection.len(), 1000);

        // Search should be fast
        let start = std::time::Instant::now();
        let query: Vec<f32> = (0..128)
            .map(|i| (500.0 * 0.1 + i as f32 * 0.01).cos())
            .collect();
        let results = collection.search(&query, 10).expect("Search failed");
        let duration = start.elapsed();

        assert_eq!(results.len(), 10);
        assert!(
            duration.as_millis() < 100,
            "Search should complete in <100ms, took {:?}",
            duration
        );
    }
}

// =============================================================================
// SCENARIO 3: Multi-Collection Workflow
// =============================================================================
// Simulates multi-tenant or multi-domain search

mod multi_collection {
    use super::*;

    fn random_embedding(seed: u64, dim: usize) -> Vec<f32> {
        (0..dim)
            .map(|i| ((seed as f32 + i as f32) * 0.123).sin())
            .collect()
    }

    #[test]
    fn test_multi_tenant_isolation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        // Create collections for different tenants
        let tenant_a = create_and_get_collection(&db, "tenant_a_docs", 256, DistanceMetric::Cosine);
        let tenant_b = create_and_get_collection(&db, "tenant_b_docs", 256, DistanceMetric::Cosine);

        // Add data to each tenant
        let points_a: Vec<Point> = (0..10)
            .map(|id| Point::without_payload(id, random_embedding(id, 256)))
            .collect();
        tenant_a.upsert(points_a).expect("Failed to upsert");

        let points_b: Vec<Point> = (100..120)
            .map(|id| Point::without_payload(id, random_embedding(id, 256)))
            .collect();
        tenant_b.upsert(points_b).expect("Failed to upsert");

        // Verify isolation
        assert_eq!(tenant_a.len(), 10);
        assert_eq!(tenant_b.len(), 20);

        // Search in tenant_a should not return tenant_b data
        let query = random_embedding(0, 256);
        let results_a = tenant_a.search(&query, 100).expect("Search failed");

        for result in &results_a {
            assert!(
                result.point.id < 100,
                "Tenant A should not see Tenant B data"
            );
        }
    }

    #[test]
    fn test_collection_lifecycle() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        // Create collection
        let collection =
            create_and_get_collection(&db, "temp_collection", 128, DistanceMetric::Euclidean);

        // Add data
        let points: Vec<Point> = (0..5)
            .map(|id| Point::without_payload(id, random_embedding(id, 128)))
            .collect();
        collection.upsert(points).expect("Failed to upsert");

        // List collections
        let collections = db.list_collections();
        assert!(collections.iter().any(|c| c == "temp_collection"));

        // Delete collection
        drop(collection);
        db.delete_collection("temp_collection")
            .expect("Failed to delete");

        // Verify deletion
        let collections_after = db.list_collections();
        assert!(!collections_after.iter().any(|c| c == "temp_collection"));
    }

    #[test]
    fn test_different_metrics_per_collection() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        // Create collections with different metrics
        let euclidean_col =
            create_and_get_collection(&db, "euclidean_vectors", 64, DistanceMetric::Euclidean);
        let dot_col = create_and_get_collection(&db, "dot_vectors", 64, DistanceMetric::DotProduct);

        // Add data (use simple normalized vectors for stability)
        for id in 0u64..10 {
            // Create simple unit vectors
            let mut embedding = vec![0.0f32; 64];
            embedding[id as usize % 64] = 1.0;

            euclidean_col
                .upsert(vec![Point::without_payload(id, embedding.clone())])
                .expect("Failed");
            dot_col
                .upsert(vec![Point::without_payload(id, embedding)])
                .expect("Failed");
        }

        // Search with query matching id=0
        let mut query = vec![0.0f32; 64];
        query[0] = 1.0;

        let euclidean_results = euclidean_col.search(&query, 5).expect("Failed");
        let dot_results = dot_col.search(&query, 5).expect("Failed");

        // All should return results
        assert_eq!(euclidean_results.len(), 5);
        assert_eq!(dot_results.len(), 5);

        // First result should be exact match (id=0)
        assert_eq!(euclidean_results[0].point.id, 0);
        assert_eq!(dot_results[0].point.id, 0);
    }
}

// =============================================================================
// SCENARIO 4: Hybrid Search (Vector + Full-Text)
// =============================================================================

mod hybrid_search {
    use super::*;

    fn text_embedding(text: &str) -> Vec<f32> {
        let hash = text.bytes().fold(0u64, |acc, b| acc.wrapping_add(b as u64));
        (0..256)
            .map(|i| ((hash as f32 + i as f32) * 0.1).sin())
            .collect()
    }

    #[test]
    fn test_vector_and_text_search() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "articles", 256, DistanceMetric::Cosine);

        // Index articles with text content
        let articles = [
            (
                1u64,
                "Rust programming language guide",
                "Learn Rust from scratch",
            ),
            (
                2,
                "Python for machine learning",
                "Introduction to ML with Python",
            ),
            (
                3,
                "Database optimization tips",
                "How to optimize your SQL queries",
            ),
            (
                4,
                "Rust async programming",
                "Understanding async/await in Rust",
            ),
            (
                5,
                "Vector search explained",
                "How vector databases power search",
            ),
        ];

        let points: Vec<Point> = articles
            .iter()
            .map(|(id, title, content)| {
                let payload = json!({
                    "title": title,
                    "content": content
                });
                Point::new(*id, text_embedding(title), Some(payload))
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        // Vector search
        let query = text_embedding("Rust programming");
        let vector_results = collection.search(&query, 3).expect("Search failed");
        assert!(!vector_results.is_empty());

        // Text search (BM25)
        let text_results = collection.text_search("rust", 10);
        assert!(
            !text_results.is_empty(),
            "Should find articles mentioning 'rust'"
        );
    }

    #[test]
    fn test_hybrid_search_ranking() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "docs", 128, DistanceMetric::Cosine);

        // Add documents with text content
        let points: Vec<Point> = (0u64..20)
            .map(|id| {
                let content = if id % 3 == 0 {
                    format!("Rust programming document {}", id)
                } else {
                    format!("Generic document number {}", id)
                };
                let embedding: Vec<f32> = (0..128)
                    .map(|i| ((id as f32 + i as f32) * 0.1).sin())
                    .collect();
                Point::new(id, embedding, Some(json!({"content": content})))
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        // Hybrid search combining vector similarity and text match
        let query_embedding: Vec<f32> = (0..128).map(|i| (0.0 + i as f32 * 0.1).sin()).collect();
        let hybrid_results = collection
            .hybrid_search(&query_embedding, "rust", 10, Some(0.5))
            .expect("Hybrid search failed");

        assert!(!hybrid_results.is_empty());
    }
}

// =============================================================================
// SCENARIO 5: Persistence and Recovery
// =============================================================================

mod persistence {
    use super::*;

    fn test_embedding(id: u64) -> Vec<f32> {
        (0..128).map(|i| (id as f32 + i as f32) * 0.01).collect()
    }

    #[test]
    fn test_collection_data_persistence() {
        // Test that data persists within a single session after flush
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection =
            create_and_get_collection(&db, "persistent_data", 128, DistanceMetric::Cosine);

        // Add data
        let points: Vec<Point> = (0u64..100)
            .map(|id| Point::without_payload(id, test_embedding(id)))
            .collect();
        collection.upsert(points).expect("Failed to upsert");

        // Flush to ensure data is persisted
        collection.flush().expect("Failed to flush");

        assert_eq!(collection.len(), 100);

        // Search should work
        let query = test_embedding(50);
        let results = collection.search(&query, 5).expect("Search failed");
        assert!(!results.is_empty());

        // First result should be exact match
        assert_eq!(results[0].point.id, 50);
    }

    #[test]
    fn test_concurrent_read_operations() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection =
            create_and_get_collection(&db, "concurrent_test", 64, DistanceMetric::Cosine);

        // Pre-populate
        let points: Vec<Point> = (0u64..100)
            .map(|id| {
                let embedding: Vec<f32> = (0..64).map(|i| (id as f32 + i as f32) * 0.01).collect();
                Point::without_payload(id, embedding)
            })
            .collect();
        collection.upsert(points).expect("Failed to upsert");

        let collection = Arc::new(collection);

        // Spawn multiple reader threads
        let mut handles = vec![];
        for thread_id in 0u64..4 {
            let col = Arc::clone(&collection);
            let handle = thread::spawn(move || {
                for _ in 0..10 {
                    let query: Vec<f32> = (0..64)
                        .map(|i| (thread_id as f32 * 10.0 + i as f32) * 0.01)
                        .collect();
                    let results = col.search(&query, 5).expect("Search failed");
                    assert!(!results.is_empty());
                }
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }
}
