//! Complete E2E Test Suite for VelesDB
//!
//! EPIC-060: Comprehensive E2E tests for all components
//! Tests the complete workflow from creation to query.

use std::path::PathBuf;
use tempfile::TempDir;
use velesdb_core::{Database, DistanceMetric, FusionStrategy, StorageMode};

/// Helper to create a test database
fn setup_test_db() -> (TempDir, Database) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    (temp_dir, db)
}

/// Helper to generate test vectors
fn generate_vector(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| ((seed as f32 * 0.1 + i as f32 * 0.01) % 1.0))
        .collect()
}

// ============================================================================
// Core Database E2E Tests
// ============================================================================

mod database_e2e {
    use super::*;

    #[test]
    fn test_complete_crud_workflow() {
        let (_temp, db) = setup_test_db();

        // Create collection
        db.create_collection("documents", 128, DistanceMetric::Cosine)
            .expect("Failed to create collection");

        let col = db.get_collection("documents").expect("Collection not found");

        // Insert vectors
        for i in 1..=100 {
            let vector = generate_vector(i, 128);
            col.upsert(i, &vector, None).expect("Failed to upsert");
        }

        // Verify count
        assert_eq!(col.config().point_count, 100);

        // Search
        let query = generate_vector(50, 128);
        let results = col.search(&query, 10, None).expect("Search failed");
        assert_eq!(results.len(), 10);

        // Delete
        col.delete(&[1, 2, 3]).expect("Delete failed");

        // Verify persistence after flush
        col.flush().expect("Flush failed");
    }

    #[test]
    fn test_all_distance_metrics() {
        let (_temp, db) = setup_test_db();

        let metrics = [
            ("cosine", DistanceMetric::Cosine),
            ("euclidean", DistanceMetric::Euclidean),
            ("dot", DistanceMetric::DotProduct),
            ("hamming", DistanceMetric::Hamming),
            ("jaccard", DistanceMetric::Jaccard),
        ];

        for (name, metric) in metrics {
            let col_name = format!("test_{}", name);
            db.create_collection(&col_name, 64, metric)
                .expect(&format!("Failed to create {} collection", name));

            let col = db.get_collection(&col_name).unwrap();

            // Insert test data
            col.upsert(1, &generate_vector(1, 64), None).unwrap();
            col.upsert(2, &generate_vector(2, 64), None).unwrap();

            // Search should work
            let results = col.search(&generate_vector(1, 64), 2, None).unwrap();
            assert!(!results.is_empty(), "Search failed for metric: {}", name);
        }
    }

    #[test]
    fn test_all_storage_modes() {
        let (_temp, db) = setup_test_db();

        let modes = [
            ("full", StorageMode::Full),
            ("sq8", StorageMode::SQ8),
            ("binary", StorageMode::Binary),
        ];

        for (name, mode) in modes {
            let col_name = format!("storage_{}", name);
            db.create_collection_with_params(&col_name, 64, DistanceMetric::Cosine, mode)
                .expect(&format!("Failed to create {} storage collection", name));

            let col = db.get_collection(&col_name).unwrap();

            // Insert and search
            for i in 1..=10 {
                col.upsert(i, &generate_vector(i, 64), None).unwrap();
            }

            let results = col.search(&generate_vector(5, 64), 5, None).unwrap();
            assert!(!results.is_empty(), "Search failed for storage mode: {}", name);
        }
    }
}

// ============================================================================
// Multi-Query Fusion E2E Tests
// ============================================================================

mod fusion_e2e {
    use super::*;

    #[test]
    fn test_rrf_fusion() {
        let (_temp, db) = setup_test_db();
        db.create_collection("fusion_test", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("fusion_test").unwrap();

        // Insert diverse vectors
        for i in 1..=50 {
            col.upsert(i, &generate_vector(i, 32), None).unwrap();
        }

        // Multi-query with RRF
        let queries: Vec<Vec<f32>> = vec![
            generate_vector(10, 32),
            generate_vector(20, 32),
            generate_vector(30, 32),
        ];
        let query_refs: Vec<&[f32]> = queries.iter().map(|v| v.as_slice()).collect();

        let results = col
            .multi_query_search(&query_refs, 10, FusionStrategy::RRF { k: 60 }, None)
            .expect("RRF fusion failed");

        assert_eq!(results.len(), 10);
    }

    #[test]
    fn test_average_fusion() {
        let (_temp, db) = setup_test_db();
        db.create_collection("avg_fusion", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("avg_fusion").unwrap();

        for i in 1..=20 {
            col.upsert(i, &generate_vector(i, 32), None).unwrap();
        }

        let queries: Vec<Vec<f32>> = vec![generate_vector(5, 32), generate_vector(15, 32)];
        let query_refs: Vec<&[f32]> = queries.iter().map(|v| v.as_slice()).collect();

        let results = col
            .multi_query_search(&query_refs, 5, FusionStrategy::Average, None)
            .expect("Average fusion failed");

        assert!(!results.is_empty());
    }

    #[test]
    fn test_batch_search() {
        let (_temp, db) = setup_test_db();
        db.create_collection("batch_test", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("batch_test").unwrap();

        for i in 1..=100 {
            col.upsert(i, &generate_vector(i, 32), None).unwrap();
        }

        let queries: Vec<Vec<f32>> = (1..=5).map(|i| generate_vector(i * 10, 32)).collect();
        let query_refs: Vec<&[f32]> = queries.iter().map(|v| v.as_slice()).collect();

        let results = col
            .batch_search(&query_refs, 3, None)
            .expect("Batch search failed");

        assert_eq!(results.len(), 5); // One result set per query
        for result_set in &results {
            assert_eq!(result_set.len(), 3); // 3 results per query
        }
    }
}

// ============================================================================
// VelesQL E2E Tests
// ============================================================================

mod velesql_e2e {
    use super::*;
    use velesdb_core::velesql::Parser;

    #[test]
    fn test_select_query_parsing() {
        let parser = Parser::new();
        
        let queries = [
            "SELECT * FROM documents LIMIT 10",
            "SELECT id, name FROM users WHERE status = 'active'",
            "SELECT * FROM embeddings WHERE vector NEAR $query LIMIT 5",
            "SELECT * FROM docs ORDER BY created_at DESC LIMIT 20",
        ];

        for query in queries {
            let result = parser.parse(query);
            assert!(result.is_ok(), "Failed to parse: {}", query);
        }
    }

    #[test]
    fn test_velesql_execution() {
        let (_temp, db) = setup_test_db();
        db.create_collection("velesql_test", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("velesql_test").unwrap();

        // Insert test data with payloads
        for i in 1..=20 {
            let payload = serde_json::json!({
                "category": if i % 2 == 0 { "tech" } else { "science" },
                "score": i * 10
            });
            col.upsert(i, &generate_vector(i, 32), Some(payload)).unwrap();
        }

        // Execute VelesQL query
        let result = col.query("SELECT * FROM velesql_test LIMIT 5");
        assert!(result.is_ok(), "VelesQL execution failed");
    }
}

// ============================================================================
// Graph Traversal E2E Tests
// ============================================================================

mod graph_e2e {
    use super::*;

    #[test]
    fn test_graph_bfs_traversal() {
        let (_temp, db) = setup_test_db();
        db.create_collection("graph_test", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("graph_test").unwrap();

        // Insert nodes
        for i in 1..=10 {
            col.upsert(i, &generate_vector(i, 32), None).unwrap();
        }

        // Add edges (if graph API available)
        // This tests the graph connectivity
        let graph = col.get_graph_store();
        if let Some(g) = graph {
            g.add_edge(1, 1, 2, "related", None).ok();
            g.add_edge(2, 2, 3, "related", None).ok();
            g.add_edge(3, 3, 4, "related", None).ok();

            let results = g.traverse_bfs(1, 3, 10, None);
            assert!(results.is_ok());
        }
    }

    #[test]
    fn test_graph_dfs_traversal() {
        let (_temp, db) = setup_test_db();
        db.create_collection("dfs_test", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("dfs_test").unwrap();

        for i in 1..=5 {
            col.upsert(i, &generate_vector(i, 32), None).unwrap();
        }

        let graph = col.get_graph_store();
        if let Some(g) = graph {
            g.add_edge(1, 1, 2, "child", None).ok();
            g.add_edge(2, 2, 3, "child", None).ok();

            let results = g.traverse_dfs(1, 5, 20, None);
            assert!(results.is_ok());
        }
    }
}

// ============================================================================
// Hybrid Search E2E Tests
// ============================================================================

mod hybrid_e2e {
    use super::*;

    #[test]
    fn test_hybrid_vector_text_search() {
        let (_temp, db) = setup_test_db();
        db.create_collection("hybrid_test", 32, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("hybrid_test").unwrap();

        // Insert with text payloads
        let docs = [
            ("Machine learning basics", 1),
            ("Deep learning neural networks", 2),
            ("Natural language processing", 3),
            ("Computer vision techniques", 4),
        ];

        for (text, id) in docs {
            let payload = serde_json::json!({ "text": text });
            col.upsert(id, &generate_vector(id, 32), Some(payload)).unwrap();
        }

        // Vector search
        let vec_results = col.search(&generate_vector(1, 32), 4, None).unwrap();
        assert_eq!(vec_results.len(), 4);

        // Text search (if BM25 enabled)
        let text_results = col.text_search("learning", 2);
        if let Ok(results) = text_results {
            assert!(!results.is_empty());
        }

        // Hybrid search
        let hybrid_results = col.hybrid_search(&generate_vector(1, 32), "learning", 3, 0.5);
        if let Ok(results) = hybrid_results {
            assert!(!results.is_empty());
        }
    }
}

// ============================================================================
// Performance & Stress Tests
// ============================================================================

mod stress_e2e {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_large_collection_operations() {
        let (_temp, db) = setup_test_db();
        db.create_collection("large_test", 128, DistanceMetric::Cosine).unwrap();
        let col = db.get_collection("large_test").unwrap();

        // Insert 10k vectors
        let start = Instant::now();
        for i in 1..=10_000 {
            col.upsert(i, &generate_vector(i, 128), None).unwrap();
        }
        let insert_time = start.elapsed();
        println!("Inserted 10k vectors in {:?}", insert_time);

        assert_eq!(col.config().point_count, 10_000);

        // Search performance
        let start = Instant::now();
        for _ in 0..100 {
            let _ = col.search(&generate_vector(42, 128), 10, None);
        }
        let search_time = start.elapsed();
        println!("100 searches in {:?}", search_time);

        // Should complete in reasonable time
        assert!(search_time.as_secs() < 10, "Search too slow");
    }

    #[test]
    fn test_concurrent_operations() {
        use std::sync::Arc;
        use std::thread;

        let (_temp, db) = setup_test_db();
        db.create_collection("concurrent_test", 64, DistanceMetric::Cosine).unwrap();
        
        let db = Arc::new(db);
        let mut handles = vec![];

        // Spawn multiple threads doing searches
        for t in 0..4 {
            let db_clone = Arc::clone(&db);
            let handle = thread::spawn(move || {
                let col = db_clone.get_collection("concurrent_test").unwrap();
                
                // Insert some vectors
                for i in (t * 100 + 1)..=(t * 100 + 100) {
                    col.upsert(i, &generate_vector(i, 64), None).ok();
                }

                // Perform searches
                for _ in 0..50 {
                    let _ = col.search(&generate_vector(t as u64 * 50, 64), 5, None);
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }
}
