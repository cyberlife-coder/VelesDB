//! Integration tests for the 10 hybrid use cases documented in `docs/guides/USE_CASES.md`.
//!
//! These tests verify that all documented `VelesQL` queries work correctly and serve
//! as living documentation for `VelesDB` capabilities.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_lossless,
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
// USE CASE 1: Contextual RAG
// =============================================================================
// Documents with REFERENCES relationships for enhanced retrieval

mod use_case_1_contextual_rag {
    use super::*;

    #[test]
    fn test_contextual_rag_basic_similarity() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "documents", 384, DistanceMetric::Cosine);

        let documents = [
            ("Quantum Computing Basics", "quantum"),
            ("Quantum Entanglement Explained", "quantum"),
            ("Machine Learning Introduction", "ml"),
            ("Deep Learning Architectures", "ml"),
            ("Classical Computing History", "computing"),
        ];

        let points: Vec<Point> = documents
            .iter()
            .enumerate()
            .map(|(id, (title, category))| {
                let mut embedding = create_mock_embedding(id as u64, 384);
                normalize(&mut embedding);
                Point::new(
                    id as u64,
                    embedding,
                    Some(json!({
                        "title": title,
                        "category": category
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut query = create_mock_embedding(0, 384);
        normalize(&mut query);

        let results = collection.search(&query, 3).expect("Search failed");

        assert!(!results.is_empty(), "Should find related documents");
        assert_eq!(results[0].point.id, 0, "First result should be exact match");
    }

    #[test]
    fn test_contextual_rag_similarity_threshold() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "documents", 128, DistanceMetric::Cosine);

        for id in 0u64..10 {
            let mut embedding = create_mock_embedding(id, 128);
            normalize(&mut embedding);
            collection
                .upsert(vec![Point::new(
                    id,
                    embedding,
                    Some(json!({"title": format!("Document {}", id)})),
                )])
                .expect("Failed to upsert");
        }

        let query_sql = "SELECT * FROM documents WHERE similarity(vector, $q) > 0.75 LIMIT 20";
        let parsed = Parser::parse(query_sql);
        assert!(parsed.is_ok(), "Query should parse: {:?}", parsed.err());
    }
}

// =============================================================================
// USE CASE 2: Expert Finder
// =============================================================================
// Multi-hop graph traversal to find experts

mod use_case_2_expert_finder {
    use super::*;

    #[test]
    fn test_expert_finder_basic_query_parses() {
        let queries = [
            "SELECT * FROM documents WHERE similarity(embedding, $query) > 0.7 AND topic = 'AI' LIMIT 10",
            "SELECT id, title, topic FROM research WHERE similarity(embedding, $q) > 0.7 LIMIT 5",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Expert finder query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_expert_finder_with_data() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "research", 256, DistanceMetric::Cosine);

        let papers = [
            (1u64, "Neural Network Optimization", "AI"),
            (2, "Quantum Computing Advances", "Quantum"),
            (3, "Deep Learning for NLP", "AI"),
            (4, "Cryptographic Protocols", "Security"),
        ];

        let points: Vec<Point> = papers
            .iter()
            .map(|(id, title, topic)| {
                let mut embedding = create_mock_embedding(*id, 256);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "title": title,
                        "topic": topic
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut query = create_mock_embedding(1, 256);
        normalize(&mut query);
        let results = collection.search(&query, 5).expect("Search failed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1);
    }
}

// =============================================================================
// USE CASE 3: Knowledge Discovery
// =============================================================================
// Variable-depth graph traversal with semantic entry point

mod use_case_3_knowledge_discovery {
    use super::*;

    #[test]
    fn test_knowledge_discovery_query_parses() {
        let queries = [
            "SELECT * FROM concepts WHERE similarity(embedding, $query) > 0.8 LIMIT 50",
            "SELECT * FROM concepts WHERE similarity(embedding, $query) > 0.8 AND category = 'technology' LIMIT 30",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Knowledge discovery query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_knowledge_discovery_basic() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection =
            create_and_get_collection(&db, "knowledge_graph", 128, DistanceMetric::Cosine);

        let concepts = [
            (1u64, "Machine Learning", "technology"),
            (2, "Neural Networks", "technology"),
            (3, "Deep Learning", "technology"),
            (4, "Biology", "science"),
            (5, "Physics", "science"),
        ];

        let points: Vec<Point> = concepts
            .iter()
            .map(|(id, name, category)| {
                let mut embedding = create_mock_embedding(*id, 128);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "name": name,
                        "category": category
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut query = create_mock_embedding(1, 128);
        normalize(&mut query);
        let results = collection.search(&query, 5).expect("Search failed");

        assert!(!results.is_empty());
    }
}

// =============================================================================
// USE CASE 4: Document Clustering
// =============================================================================
// Aggregations with similarity filtering

mod use_case_4_document_clustering {
    use super::*;

    #[test]
    fn test_clustering_query_parses() {
        let queries = [
            "SELECT category, COUNT(*) FROM documents WHERE similarity(embedding, $query) > 0.6 GROUP BY category ORDER BY COUNT(*) DESC LIMIT 10",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Clustering query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_clustering_basic_data() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "documents", 128, DistanceMetric::Cosine);

        let categories = ["tech", "science", "tech", "science", "tech", "arts", "tech"];
        let points: Vec<Point> = categories
            .iter()
            .enumerate()
            .map(|(id, category)| {
                let mut embedding = create_mock_embedding(id as u64, 128);
                normalize(&mut embedding);
                Point::new(id as u64, embedding, Some(json!({"category": category})))
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");
        assert_eq!(collection.len(), 7);

        let mut query = create_mock_embedding(0, 128);
        normalize(&mut query);
        let results = collection.search(&query, 10).expect("Search failed");
        assert!(!results.is_empty());
    }
}

// =============================================================================
// USE CASE 5: Semantic Search with Filters
// =============================================================================
// Vector NEAR with metadata filters

mod use_case_5_semantic_search_filters {
    use super::*;

    #[test]
    fn test_semantic_filter_query_parses() {
        let queries = [
            "SELECT id, title, score FROM articles WHERE vector NEAR $query AND category IN ('technology', 'science', 'engineering') AND published_date >= '2024-01-01' AND access_level = 'public' LIMIT 20 WITH (mode = 'balanced')",
            "SELECT id, title FROM articles WHERE similarity(embedding, $query) > 0.75 AND category = 'technology' AND published_date >= '2024-01-01' ORDER BY similarity(embedding, $query) DESC LIMIT 20",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Semantic filter query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_semantic_search_with_metadata() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "articles", 256, DistanceMetric::Cosine);

        let articles = [
            (
                1u64,
                "AI Advances 2024",
                "technology",
                "2024-03-15",
                "public",
            ),
            (2, "Private Research", "technology", "2024-02-01", "private"),
            (3, "Open Science Paper", "science", "2024-01-20", "public"),
            (4, "Old Tech Article", "technology", "2023-06-01", "public"),
            (
                5,
                "Latest Engineering",
                "engineering",
                "2024-04-01",
                "public",
            ),
        ];

        let points: Vec<Point> = articles
            .iter()
            .map(|(id, title, category, date, access)| {
                let mut embedding = create_mock_embedding(*id, 256);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "title": title,
                        "category": category,
                        "published_date": date,
                        "access_level": access
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut query = create_mock_embedding(1, 256);
        normalize(&mut query);
        let results = collection.search(&query, 10).expect("Search failed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1);
    }
}

// =============================================================================
// USE CASE 6: Recommendation Engine
// =============================================================================
// User-item graph for personalization

mod use_case_6_recommendation_engine {
    use super::*;

    #[test]
    fn test_recommendation_query_parses() {
        let queries = [
            "SELECT * FROM items WHERE similarity(embedding, $preference) > 0.7 LIMIT 20",
            "SELECT id, name, category FROM items WHERE similarity(embedding, $pref) > 0.7 AND category = 'electronics' LIMIT 10",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Recommendation query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_recommendation_basic_flow() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "items", 128, DistanceMetric::Cosine);

        let items = [
            (1u64, "Wireless Headphones", "electronics", 79.99f32),
            (2, "USB Cable", "electronics", 12.99),
            (3, "Running Shoes", "sports", 129.99),
            (4, "Yoga Mat", "sports", 39.99),
            (5, "Smart Watch", "electronics", 299.99),
        ];

        let points: Vec<Point> = items
            .iter()
            .map(|(id, name, category, price)| {
                let mut embedding = create_mock_embedding(*id, 128);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "name": name,
                        "category": category,
                        "price": price
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut preference = create_mock_embedding(1, 128);
        normalize(&mut preference);
        let recommendations = collection.search(&preference, 5).expect("Search failed");

        assert!(!recommendations.is_empty());
    }
}

// =============================================================================
// USE CASE 7: Entity Resolution
// =============================================================================
// Deduplication using high similarity threshold

mod use_case_7_entity_resolution {
    use super::*;

    #[test]
    fn test_entity_resolution_query_parses() {
        let queries = [
            "SELECT id, name FROM companies WHERE similarity(embedding, $new_entity_embedding) > 0.95 LIMIT 5",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Entity resolution query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_entity_resolution_deduplication() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "companies", 128, DistanceMetric::Cosine);

        let companies = [
            (1u64, "OpenAI", "AI research company"),
            (2, "Google", "Technology company"),
            (3, "Microsoft", "Software company"),
        ];

        let points: Vec<Point> = companies
            .iter()
            .map(|(id, name, description)| {
                let mut embedding = create_mock_embedding(*id, 128);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "name": name,
                        "description": description
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut query = create_mock_embedding(1, 128);
        normalize(&mut query);
        let results = collection.search(&query, 1).expect("Search failed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 1);
        assert!(
            results[0].score > 0.99,
            "Exact match should have very high score"
        );
    }
}

// =============================================================================
// USE CASE 8: Trend Analysis
// =============================================================================
// Temporal aggregations on semantically filtered data

mod use_case_8_trend_analysis {
    use super::*;

    #[test]
    fn test_trend_analysis_query_parses() {
        let queries = [
            "SELECT category, COUNT(*) FROM articles WHERE similarity(embedding, $query) > 0.6 AND published_at BETWEEN '2024-01-01' AND '2024-12-31' GROUP BY category HAVING COUNT(*) > 10 ORDER BY COUNT(*) DESC",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Trend analysis query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_trend_analysis_basic() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "articles", 128, DistanceMetric::Cosine);

        let articles: Vec<Point> = (0u64..20)
            .map(|id| {
                let mut embedding = create_mock_embedding(id, 128);
                normalize(&mut embedding);
                let month = (id % 12) + 1;
                Point::new(
                    id,
                    embedding,
                    Some(json!({
                        "title": format!("Article {}", id),
                        "published_at": format!("2024-{:02}-15", month),
                        "category": if id % 3 == 0 { "tech" } else { "science" }
                    })),
                )
            })
            .collect();

        collection.upsert(articles).expect("Failed to upsert");
        assert_eq!(collection.len(), 20);
    }
}

// =============================================================================
// USE CASE 9: Impact Analysis
// =============================================================================
// Multi-hop graph traversal for dependency tracking

mod use_case_9_impact_analysis {
    use super::*;

    #[test]
    fn test_impact_analysis_query_parses() {
        let queries = [
            "SELECT * FROM components WHERE similarity(embedding, $pattern) > 0.6 AND criticality = 'critical' LIMIT 100",
            "SELECT id, name, criticality FROM components WHERE similarity(embedding, $pattern) > 0.6 ORDER BY criticality DESC LIMIT 50",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Impact analysis query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_impact_analysis_components() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "components", 64, DistanceMetric::Cosine);

        let components = [
            (1u64, "auth-service", "service", "critical"),
            (2, "user-api", "api", "high"),
            (3, "database", "storage", "critical"),
            (4, "cache", "storage", "medium"),
            (5, "frontend", "ui", "low"),
        ];

        let points: Vec<Point> = components
            .iter()
            .map(|(id, name, comp_type, criticality)| {
                let mut embedding = create_mock_embedding(*id, 64);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "name": name,
                        "type": comp_type,
                        "criticality": criticality
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");
        assert_eq!(collection.len(), 5);
    }
}

// =============================================================================
// USE CASE 10: Conversational Memory
// =============================================================================
// Agent memory pattern with message graph

mod use_case_10_conversational_memory {
    use super::*;

    #[test]
    fn test_conversational_memory_query_parses() {
        let queries = [
            "SELECT content, role, timestamp FROM messages WHERE conversation_id = $conv_id AND similarity(embedding, $query) > 0.6 ORDER BY timestamp DESC LIMIT 10",
            "SELECT * FROM messages WHERE user_id = $user_id AND similarity(embedding, $query) > 0.7 ORDER BY timestamp DESC LIMIT 20",
        ];

        for query in queries {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Conversational memory query should parse: {} - {:?}",
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_conversational_memory_storage() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "messages", 128, DistanceMetric::Cosine);

        let messages = [
            (1u64, "user", "What is VelesDB?", "conv_001"),
            (
                2,
                "assistant",
                "VelesDB is a vector database...",
                "conv_001",
            ),
            (3, "user", "How do I create a collection?", "conv_001"),
            (
                4,
                "assistant",
                "You can create a collection using...",
                "conv_001",
            ),
            (5, "user", "Thanks!", "conv_001"),
        ];

        let points: Vec<Point> = messages
            .iter()
            .map(|(id, role, content, conv_id)| {
                let mut embedding = create_mock_embedding(*id, 128);
                normalize(&mut embedding);
                Point::new(
                    *id,
                    embedding,
                    Some(json!({
                        "role": role,
                        "content": content,
                        "conversation_id": conv_id,
                        "timestamp": format!("2024-01-15T10:{:02}:00Z", id)
                    })),
                )
            })
            .collect();

        collection.upsert(points).expect("Failed to upsert");

        let mut query = create_mock_embedding(3, 128);
        normalize(&mut query);
        let results = collection.search(&query, 3).expect("Search failed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 3, "Should find exact match first");
    }

    #[test]
    fn test_agent_memory_retrieval_flow() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection =
            create_and_get_collection(&db, "agent_memory", 128, DistanceMetric::Cosine);

        for id in 0u64..50 {
            let mut embedding = create_mock_embedding(id, 128);
            normalize(&mut embedding);
            collection
                .upsert(vec![Point::new(
                    id,
                    embedding,
                    Some(json!({
                        "user_id": "user123",
                        "role": if id % 2 == 0 { "user" } else { "assistant" },
                        "content": format!("Message content {}", id),
                        "timestamp": format!("2024-01-15T{:02}:{:02}:00Z", id / 60, id % 60)
                    })),
                )])
                .expect("Failed to upsert");
        }

        let mut query = create_mock_embedding(25, 128);
        normalize(&mut query);
        let context = collection.search(&query, 10).expect("Search failed");

        assert_eq!(context.len(), 10);
        assert_eq!(context[0].point.id, 25);
    }
}

// =============================================================================
// CROSS-USE-CASE TESTS
// =============================================================================

mod cross_use_case {
    use super::*;

    #[test]
    fn test_all_documented_queries_parse() {
        let documented_queries = [
            "SELECT * FROM docs WHERE similarity(vector, $q) > 0.8",
            "SELECT * FROM docs WHERE vector NEAR $v LIMIT 10",
            "SELECT * FROM docs WHERE vector NEAR $v AND category = 'tech' AND price > 50 LIMIT 10",
            "SELECT * FROM documents WHERE similarity(embedding, $query) > 0.75 LIMIT 20",
            "SELECT category, COUNT(*) FROM documents WHERE similarity(embedding, $query) > 0.6 GROUP BY category ORDER BY COUNT(*) DESC LIMIT 10",
        ];

        for (i, query) in documented_queries.iter().enumerate() {
            let parsed = Parser::parse(query);
            assert!(
                parsed.is_ok(),
                "Documented query {} should parse: {} - {:?}",
                i + 1,
                query,
                parsed.err()
            );
        }
    }

    #[test]
    fn test_combined_vector_and_filter_search() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let db = Database::open(temp_dir.path()).expect("Failed to open database");

        let collection = create_and_get_collection(&db, "hybrid_test", 64, DistanceMetric::Cosine);

        for id in 0u64..100 {
            let mut embedding = create_mock_embedding(id, 64);
            normalize(&mut embedding);
            collection
                .upsert(vec![Point::new(
                    id,
                    embedding,
                    Some(json!({
                        "category": if id % 3 == 0 { "A" } else if id % 3 == 1 { "B" } else { "C" },
                        "score": id as f64 / 100.0
                    })),
                )])
                .expect("Failed to upsert");
        }

        let mut query = create_mock_embedding(50, 64);
        normalize(&mut query);
        let results = collection.search(&query, 20).expect("Search failed");

        assert!(!results.is_empty());
        assert_eq!(results[0].point.id, 50);
    }
}
