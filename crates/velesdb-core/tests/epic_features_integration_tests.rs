//! Integration tests for completed EPICs features.
//!
//! These tests verify end-to-end functionality of features marked as DONE in EPICs.
//! Coverage: EPIC-005, EPIC-009, EPIC-017, EPIC-021, EPIC-028, EPIC-031

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::uninlined_format_args
)]

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{velesql::Parser, Database, DistanceMetric, Point};

fn create_mock_embedding(seed: u64, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|i| (seed as f32 * 0.1 + i as f32 * 0.01).sin())
        .collect()
}

fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

// =============================================================================
// EPIC-005: VelesQL MATCH Clause (Graph Pattern Matching)
// =============================================================================

mod epic_005_match_clause {
    use super::*;

    #[test]
    fn test_match_clause_basic_pattern_parses() {
        let queries = [
            "SELECT * FROM docs WHERE id MATCH 'pattern'",
            "SELECT * FROM articles WHERE title MATCH 'rust programming'",
            "SELECT * FROM products WHERE description MATCH 'wireless bluetooth'",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "MATCH query should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_match_with_similarity_hybrid() {
        let query = "SELECT * FROM docs WHERE title MATCH 'database' AND similarity(embedding, $v) > 0.7 LIMIT 10";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Hybrid MATCH+similarity should parse");
    }

    #[test]
    fn test_match_with_filters() {
        let query = "SELECT * FROM products WHERE name MATCH 'laptop' AND price > 500 AND category = 'electronics' LIMIT 20";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "MATCH with filters should parse");
    }
}

// =============================================================================
// EPIC-009: Graph Property Index
// =============================================================================

mod epic_009_property_index {
    use super::*;

    #[test]
    fn test_create_and_use_property_index() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        db.create_collection("indexed_docs", 128, DistanceMetric::Cosine)
            .expect("Failed to create collection");
        let collection = db
            .get_collection("indexed_docs")
            .expect("Failed to get collection");

        // Create property index for O(1) lookups
        collection
            .create_property_index("Document", "category")
            .expect("Failed to create property index");

        // Verify index exists
        assert!(collection.has_property_index("Document", "category"));

        // List indexes
        let indexes = collection.list_indexes();
        assert!(!indexes.is_empty(), "Should have at least one index");
    }

    #[test]
    fn test_range_index_creation() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        db.create_collection("events", 64, DistanceMetric::Cosine)
            .expect("Failed to create collection");
        let collection = db
            .get_collection("events")
            .expect("Failed to get collection");

        // Create range index for O(log n) range queries
        collection
            .create_range_index("Event", "timestamp")
            .expect("Failed to create range index");

        assert!(collection.has_range_index("Event", "timestamp"));
    }

    #[test]
    fn test_drop_index() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        db.create_collection("test_drop", 64, DistanceMetric::Cosine)
            .expect("Failed to create collection");
        let collection = db
            .get_collection("test_drop")
            .expect("Failed to get collection");

        collection
            .create_property_index("Node", "name")
            .expect("Failed to create index");
        assert!(collection.has_property_index("Node", "name"));

        let dropped = collection
            .drop_index("Node", "name")
            .expect("Failed to drop");
        assert!(dropped, "Should return true when index dropped");
        assert!(!collection.has_property_index("Node", "name"));
    }
}

// =============================================================================
// EPIC-017: Aggregations (GROUP BY, COUNT, SUM, AVG, MIN, MAX)
// =============================================================================

mod epic_017_aggregations {
    use super::*;

    #[test]
    fn test_aggregation_queries_parse() {
        let queries = [
            "SELECT category, COUNT(*) FROM products GROUP BY category",
            "SELECT department, AVG(salary) FROM employees GROUP BY department",
            "SELECT region, SUM(sales), MIN(sales), MAX(sales) FROM orders GROUP BY region",
            "SELECT category, COUNT(*) FROM products GROUP BY category HAVING COUNT(*) > 5",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "Aggregation query should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_aggregation_with_order_by() {
        let query = "SELECT category, COUNT(*) AS cnt FROM products GROUP BY category ORDER BY COUNT(*) DESC LIMIT 10";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Aggregation with ORDER BY should parse");
    }

    #[test]
    fn test_having_clause_with_logical_operators() {
        let queries = [
            "SELECT category, COUNT(*) FROM items GROUP BY category HAVING COUNT(*) > 10 AND AVG(price) < 100",
            "SELECT region, SUM(amount) FROM sales GROUP BY region HAVING SUM(amount) > 1000 OR COUNT(*) > 50",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "HAVING with logical operators should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }
}

// =============================================================================
// EPIC-021: VelesQL JOIN (Cross-Store Queries)
// =============================================================================

mod epic_021_join {
    use super::*;

    #[test]
    fn test_join_queries_parse() {
        let queries = [
            "SELECT * FROM orders JOIN customers ON orders.customer_id = customers.id",
            "SELECT * FROM products LEFT JOIN inventory ON products.sku = inventory.sku",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "JOIN query should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_join_with_alias() {
        // JOIN with alias uses different syntax
        let query = "SELECT * FROM orders JOIN customers AS c ON orders.customer_id = c.id";
        let result = Parser::parse(query);
        assert!(
            result.is_ok(),
            "JOIN with alias should parse: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_using_clause() {
        let query = "SELECT * FROM orders JOIN order_items USING (order_id)";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "USING clause should parse");
    }

    #[test]
    fn test_multiple_join_types() {
        let queries = [
            "SELECT * FROM a INNER JOIN b ON a.id = b.a_id",
            "SELECT * FROM a LEFT OUTER JOIN b ON a.id = b.a_id",
            "SELECT * FROM a RIGHT JOIN b ON a.id = b.a_id",
            "SELECT * FROM a FULL OUTER JOIN b ON a.id = b.a_id",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "JOIN type should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }
}

// =============================================================================
// EPIC-028: ORDER BY Multi-Columns
// =============================================================================

mod epic_028_orderby_multi {
    use super::*;

    #[test]
    fn test_orderby_multiple_columns() {
        let queries = [
            "SELECT * FROM products ORDER BY category ASC, price DESC",
            "SELECT * FROM employees ORDER BY department, salary DESC, name ASC",
            "SELECT * FROM logs ORDER BY timestamp DESC, severity ASC LIMIT 100",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "Multi-column ORDER BY should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_orderby_with_similarity() {
        let query = "SELECT * FROM docs ORDER BY similarity(embedding, $query) DESC, created_at DESC LIMIT 20";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "ORDER BY similarity + column should parse");
    }

    #[test]
    fn test_orderby_aggregates() {
        let query = "SELECT category, COUNT(*) FROM products GROUP BY category ORDER BY COUNT(*) DESC, category ASC";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "ORDER BY aggregates should parse");
    }
}

// =============================================================================
// EPIC-031: Multimodel Query Engine
// =============================================================================

mod epic_031_multimodel {
    use super::*;

    #[test]
    fn test_vector_near_with_filters() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        db.create_collection("multimodel", 64, DistanceMetric::Cosine)
            .expect("Failed to create collection");
        let collection = db.get_collection("multimodel").expect("Failed to get");

        // Insert data with varied metadata
        for id in 0u64..50 {
            let mut embedding = create_mock_embedding(id, 64);
            normalize(&mut embedding);
            let category = match id % 3 {
                0 => "A",
                1 => "B",
                _ => "C",
            };
            collection
                .upsert(vec![Point::new(
                    id,
                    embedding,
                    Some(json!({"category": category, "score": id as f64 / 50.0})),
                )])
                .expect("Failed to upsert");
        }

        // Vector search
        let mut query = create_mock_embedding(25, 64);
        normalize(&mut query);
        let results = collection.search(&query, 10).expect("Search failed");

        assert_eq!(results.len(), 10);
        assert_eq!(
            results[0].point.id, 25,
            "First result should be exact match"
        );
    }

    #[test]
    fn test_similarity_function_in_query() {
        let query = "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 AND category = 'tech' ORDER BY similarity(embedding, $v) DESC LIMIT 20";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "similarity() function query should parse");
    }

    #[test]
    fn test_multi_query_search_syntax() {
        let query = "SELECT * FROM docs WHERE vector NEAR_FUSED [$v1, $v2, $v3] USING FUSION 'rrf' (k = 60) LIMIT 10";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "NEAR_FUSED multi-vector query should parse");
    }
}

// =============================================================================
// EPIC-040: VelesQL Language V2 (Set Operations, Fusion)
// =============================================================================

mod epic_040_velesql_v2 {
    use super::*;

    #[test]
    fn test_union_queries() {
        let queries = [
            "SELECT * FROM table1 UNION SELECT * FROM table2",
            "SELECT * FROM active_users UNION ALL SELECT * FROM inactive_users",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "UNION query should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_intersect_except() {
        let queries = [
            "SELECT * FROM set1 INTERSECT SELECT * FROM set2",
            "SELECT * FROM all_users EXCEPT SELECT * FROM banned_users",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "Set operation should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_fusion_clause() {
        let query = "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 USING FUSION(strategy = 'rrf', k = 60)";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "USING FUSION clause should parse");
    }

    #[test]
    fn test_with_clause_options() {
        let queries = [
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (mode = 'accurate')",
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10 WITH (ef_search = 512, timeout_ms = 5000)",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "WITH clause should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }
}

// =============================================================================
// EPIC-044: VelesQL Robustness (Error Handling, Edge Cases)
// =============================================================================

mod epic_044_robustness {
    use super::*;

    #[test]
    fn test_quoted_identifiers() {
        let queries = [
            "SELECT * FROM `select` WHERE `from` = 'value'",
            r#"SELECT * FROM "order" WHERE "group" = 'test'"#,
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "Quoted identifiers should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_complex_nested_conditions() {
        let query = "SELECT * FROM items WHERE (category = 'A' OR category = 'B') AND (price > 10 AND price < 100) AND status = 'active'";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Complex nested conditions should parse");
    }

    #[test]
    fn test_ilike_case_insensitive() {
        let queries = [
            "SELECT * FROM users WHERE name LIKE '%john%'",
            "SELECT * FROM users WHERE name ILIKE '%john%'",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "LIKE/ILIKE should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }

    #[test]
    fn test_between_clause() {
        let query = "SELECT * FROM events WHERE timestamp BETWEEN '2024-01-01' AND '2024-12-31'";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "BETWEEN clause should parse");
    }

    #[test]
    fn test_is_null_conditions() {
        let queries = [
            "SELECT * FROM users WHERE email IS NULL",
            "SELECT * FROM users WHERE email IS NOT NULL",
        ];

        for query in queries {
            let result = Parser::parse(query);
            assert!(
                result.is_ok(),
                "IS NULL should parse: {} - {:?}",
                query,
                result.err()
            );
        }
    }
}

// =============================================================================
// Cross-EPIC Integration Tests
// =============================================================================

mod cross_epic_integration {
    use super::*;

    #[test]
    fn test_full_featured_query() {
        // Combines: similarity, filters, aggregation, ORDER BY, WITH clause
        let query = "SELECT category, COUNT(*) FROM products WHERE similarity(embedding, $v) > 0.6 AND price > 50 GROUP BY category HAVING COUNT(*) > 3 ORDER BY COUNT(*) DESC LIMIT 10 WITH (mode = 'accurate')";
        let result = Parser::parse(query);
        assert!(result.is_ok(), "Full-featured query should parse");
    }

    #[test]
    fn test_hybrid_search_end_to_end() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        db.create_collection("hybrid_test", 128, DistanceMetric::Cosine)
            .expect("Failed to create collection");
        let collection = db.get_collection("hybrid_test").expect("Failed to get");

        // Insert documents with text and embeddings
        let docs = [
            (1u64, "Rust programming guide", "tech"),
            (2, "Python machine learning", "tech"),
            (3, "Cooking recipes", "food"),
            (4, "Database optimization", "tech"),
            (5, "Travel destinations", "travel"),
        ];

        for (id, title, category) in docs {
            let mut embedding = create_mock_embedding(id, 128);
            normalize(&mut embedding);
            collection
                .upsert(vec![Point::new(
                    id,
                    embedding,
                    Some(json!({"title": title, "category": category})),
                )])
                .expect("Failed to upsert");
        }

        // Vector search
        let mut query = create_mock_embedding(1, 128);
        normalize(&mut query);
        let vector_results = collection.search(&query, 3).expect("Vector search failed");
        assert!(!vector_results.is_empty());

        // Text search
        let text_results = collection.text_search("programming", 10);
        assert!(!text_results.is_empty(), "Text search should find results");

        // Hybrid search
        let hybrid_results = collection
            .hybrid_search(&query, "programming", 5, Some(0.5))
            .expect("Hybrid search failed");
        assert!(!hybrid_results.is_empty());
    }
}
