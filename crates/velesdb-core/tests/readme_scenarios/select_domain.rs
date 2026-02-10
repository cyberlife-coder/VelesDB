//! SELECT domain scenario tests (Scenarios 1-3) — Tests created by Phase 4 Plan 02.
//!
//! Validates that SELECT + NEAR + column filter queries (LIKE, BETWEEN, temporal,
//! multi-ORDER BY) work as documented in README Scenarios 1-3.

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::velesql::Parser;
use velesdb_core::DistanceMetric;

use crate::helpers;

/// Converts a f32 embedding vector to a JSON array for query parameters.
fn embedding_to_json_param(vec: &[f32]) -> serde_json::Value {
    serde_json::Value::Array(
        vec.iter()
            .map(|&f| serde_json::Value::from(f64::from(f)))
            .collect(),
    )
}

// ============================================================================
// Scenario 1: Medical Research Assistant
// ============================================================================

/// README query:
/// ```sql
/// SELECT study_id, title, publication_date
/// FROM medical_studies
/// WHERE vector NEAR $cancer_research_embedding
///   AND content LIKE '%BRCA1%'
///   AND publication_date > '2025-01-01'
/// ORDER BY similarity() DESC
/// LIMIT 5
/// ```
/// 8 medical studies: mix of BRCA1/non-BRCA1 content and pre/post-2025 dates.
fn scenario1_medical_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            1,
            helpers::generate_embedding(10, 128),
            json!({"study_id": "S001", "title": "BRCA1 Gene Mutations in Cancer", "content": "Study on BRCA1 gene mutations and their role in breast cancer", "publication_date": "2025-03-15"}),
        ),
        (
            2,
            helpers::generate_embedding(11, 128),
            json!({"study_id": "S002", "title": "mRNA Vaccine Development", "content": "Novel mRNA approaches for respiratory diseases", "publication_date": "2025-06-01"}),
        ),
        (
            3,
            helpers::generate_embedding(12, 128),
            json!({"study_id": "S003", "title": "BRCA1 Targeted Therapy", "content": "Targeted therapies for BRCA1 mutation carriers", "publication_date": "2025-01-20"}),
        ),
        (
            4,
            helpers::generate_embedding(13, 128),
            json!({"study_id": "S004", "title": "Cardiovascular Risk Factors", "content": "Analysis of heart disease risk factors in populations", "publication_date": "2025-04-10"}),
        ),
        (
            5,
            helpers::generate_embedding(14, 128),
            json!({"study_id": "S005", "title": "BRCA1 and Ovarian Cancer", "content": "Correlation between BRCA1 mutations and ovarian cancer risk", "publication_date": "2024-06-15"}),
        ),
        (
            6,
            helpers::generate_embedding(15, 128),
            json!({"study_id": "S006", "title": "Immunotherapy Advances", "content": "PD-1 checkpoint inhibitors in oncology", "publication_date": "2024-11-20"}),
        ),
        (
            7,
            helpers::generate_embedding(16, 128),
            json!({"study_id": "S007", "title": "BRCA1 Expression Patterns", "content": "Expression of BRCA1 in normal and tumor tissues", "publication_date": "2025-07-01"}),
        ),
        (
            8,
            helpers::generate_embedding(17, 128),
            json!({"study_id": "S008", "title": "Genomic Sequencing Methods", "content": "Next-generation sequencing of cancer genomes", "publication_date": "2024-03-01"}),
        ),
    ]
}

#[test]
fn test_scenario1_medical_research() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "medical_studies", 128, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &scenario1_medical_nodes());

    let query_str = "\
        SELECT study_id, title, publication_date \
        FROM medical_studies \
        WHERE vector NEAR $cancer_research_embedding \
          AND content LIKE '%BRCA1%' \
          AND publication_date > '2025-01-01' \
        ORDER BY similarity(vector, $cancer_research_embedding) DESC \
        LIMIT 5";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let query_vec = helpers::generate_embedding(10, 128);
    let mut params = HashMap::new();
    params.insert(
        "cancer_research_embedding".to_string(),
        embedding_to_json_param(&query_vec),
    );

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Query should execute");

    // Matching: S001 (BRCA1, 2025-03-15), S003 (BRCA1, 2025-01-20), S007 (BRCA1, 2025-07-01)
    // Excluded: S002/S004 (no BRCA1), S005 (BRCA1 but old), S006/S008 (no BRCA1 + old)
    assert_eq!(
        results.len(),
        3,
        "Should return 3 results (BRCA1 + post-2025), got {}",
        results.len()
    );

    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");

        let content = payload
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            content.contains("BRCA1"),
            "Content should contain 'BRCA1', got: {content}"
        );

        let pub_date = payload
            .get("publication_date")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            pub_date > "2025-01-01",
            "Date '{pub_date}' should be > '2025-01-01'"
        );
    }

    // ORDER BY similarity() DESC — scores non-increasing
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Similarity DESC: {:.4} >= {:.4}",
            window[0].score,
            window[1].score
        );
    }

    assert!(results.len() <= 5, "LIMIT 5 should be respected");
}

// ============================================================================
// Scenario 2: E-commerce Recommendation
// ============================================================================

/// README query:
/// ```sql
/// SELECT product_id, name, price
/// FROM products
/// WHERE vector NEAR $user_preferences
///   AND price BETWEEN 20.00 AND 100.00
///   AND category = 'electronics'
/// ORDER BY similarity() DESC, price ASC
/// LIMIT 8
/// ```
/// 12 products: mix of categories and price ranges.
fn scenario2_product_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            1,
            helpers::generate_embedding(20, 128),
            json!({"product_id": "P001", "name": "Wireless Mouse", "price": 29.99, "category": "electronics"}),
        ),
        (
            2,
            helpers::generate_embedding(21, 128),
            json!({"product_id": "P002", "name": "USB-C Hub", "price": 49.99, "category": "electronics"}),
        ),
        (
            3,
            helpers::generate_embedding(22, 128),
            json!({"product_id": "P003", "name": "4K Monitor", "price": 399.99, "category": "electronics"}),
        ),
        (
            4,
            helpers::generate_embedding(23, 128),
            json!({"product_id": "P004", "name": "Bluetooth Speaker", "price": 79.99, "category": "electronics"}),
        ),
        (
            5,
            helpers::generate_embedding(24, 128),
            json!({"product_id": "P005", "name": "Running Shoes", "price": 89.99, "category": "sports"}),
        ),
        (
            6,
            helpers::generate_embedding(25, 128),
            json!({"product_id": "P006", "name": "Yoga Mat", "price": 35.00, "category": "sports"}),
        ),
        (
            7,
            helpers::generate_embedding(26, 128),
            json!({"product_id": "P007", "name": "Coffee Table", "price": 149.99, "category": "home"}),
        ),
        (
            8,
            helpers::generate_embedding(27, 128),
            json!({"product_id": "P008", "name": "Desk Lamp", "price": 45.00, "category": "home"}),
        ),
        (
            9,
            helpers::generate_embedding(28, 128),
            json!({"product_id": "P009", "name": "Earbuds", "price": 59.99, "category": "electronics"}),
        ),
        (
            10,
            helpers::generate_embedding(29, 128),
            json!({"product_id": "P010", "name": "Phone Case", "price": 15.99, "category": "electronics"}),
        ),
        (
            11,
            helpers::generate_embedding(30, 128),
            json!({"product_id": "P011", "name": "Keyboard", "price": 89.99, "category": "electronics"}),
        ),
        (
            12,
            helpers::generate_embedding(31, 128),
            json!({"product_id": "P012", "name": "Webcam", "price": 69.99, "category": "electronics"}),
        ),
    ]
}

#[test]
fn test_scenario2_ecommerce() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "products", 128, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &scenario2_product_nodes());

    let query_str = "\
        SELECT product_id, name, price \
        FROM products \
        WHERE vector NEAR $user_preferences \
          AND price BETWEEN 20.0 AND 100.0 \
          AND category = 'electronics' \
        ORDER BY similarity(vector, $user_preferences) DESC, price ASC \
        LIMIT 8";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let query_vec = helpers::generate_embedding(20, 128);
    let mut params = HashMap::new();
    params.insert(
        "user_preferences".to_string(),
        embedding_to_json_param(&query_vec),
    );

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Query should execute");

    // Matching: P001(29.99), P002(49.99), P004(79.99), P009(59.99), P011(89.99), P012(69.99)
    // Excluded: P003(too expensive), P005/P006(sports), P007/P008(home), P010(too cheap)
    assert_eq!(
        results.len(),
        6,
        "Should return 6 electronics in price range, got {}",
        results.len()
    );

    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");

        let price = payload
            .get("price")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        assert!(
            (20.0..=100.0).contains(&price),
            "Price {price} should be between 20.00 and 100.00"
        );

        let category = payload
            .get("category")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert_eq!(
            category, "electronics",
            "Category should be 'electronics', got '{category}'"
        );
    }

    // ORDER BY similarity() DESC — primary sort by similarity
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Similarity DESC: {:.4} >= {:.4}",
            window[0].score,
            window[1].score
        );
    }

    assert!(results.len() <= 8, "LIMIT 8 should be respected");
}

// ============================================================================
// Scenario 3: Cybersecurity Threat Detection
// ============================================================================

/// README query (adapted for deterministic testing with fixed epoch cutoff):
/// ```sql
/// SELECT malware_hash, threat_level, first_seen
/// FROM threat_intel
/// WHERE vector NEAR $current_threat_embedding
///   AND first_seen > 1735689600
///   AND threat_level > 0.8
/// ORDER BY similarity() DESC, first_seen DESC
/// LIMIT 10
/// ```
///
/// Note: Original README uses `NOW() - INTERVAL '7 days'`. We use a fixed epoch
/// (1735689600 = 2025-01-01) for deterministic testing. Temporal expression
/// parsing is validated separately in Phase 1 tests.
/// 10 threats: varying threat levels and timestamps.
/// Cutoff: 1735689600 (2025-01-01 00:00:00 UTC)
#[allow(clippy::unreadable_literal)] // Reason: epoch timestamps are standard format
fn scenario3_threat_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            1,
            helpers::generate_embedding(40, 128),
            json!({"malware_hash": "abc123", "threat_level": 0.95, "first_seen": 1738000000_i64}),
        ),
        (
            2,
            helpers::generate_embedding(41, 128),
            json!({"malware_hash": "def456", "threat_level": 0.85, "first_seen": 1738100000_i64}),
        ),
        (
            3,
            helpers::generate_embedding(42, 128),
            json!({"malware_hash": "ghi789", "threat_level": 0.50, "first_seen": 1738200000_i64}),
        ),
        (
            4,
            helpers::generate_embedding(43, 128),
            json!({"malware_hash": "jkl012", "threat_level": 0.92, "first_seen": 1735000000_i64}),
        ),
        (
            5,
            helpers::generate_embedding(44, 128),
            json!({"malware_hash": "mno345", "threat_level": 0.30, "first_seen": 1737900000_i64}),
        ),
        (
            6,
            helpers::generate_embedding(45, 128),
            json!({"malware_hash": "pqr678", "threat_level": 0.88, "first_seen": 1738300000_i64}),
        ),
        (
            7,
            helpers::generate_embedding(46, 128),
            json!({"malware_hash": "stu901", "threat_level": 0.96, "first_seen": 1700000000_i64}),
        ),
        (
            8,
            helpers::generate_embedding(47, 128),
            json!({"malware_hash": "vwx234", "threat_level": 0.91, "first_seen": 1738050000_i64}),
        ),
        (
            9,
            helpers::generate_embedding(48, 128),
            json!({"malware_hash": "yza567", "threat_level": 0.60, "first_seen": 1730000000_i64}),
        ),
        (
            10,
            helpers::generate_embedding(49, 128),
            json!({"malware_hash": "bcd890", "threat_level": 0.89, "first_seen": 1738400000_i64}),
        ),
    ]
}

#[test]
fn test_scenario3_cybersecurity() {
    let (_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "threat_intel", 128, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &scenario3_threat_nodes());

    let query_str = "\
        SELECT malware_hash, threat_level, first_seen \
        FROM threat_intel \
        WHERE vector NEAR $current_threat_embedding \
          AND first_seen > 1735689600 \
          AND threat_level > 0.8 \
        ORDER BY similarity(vector, $current_threat_embedding) DESC, first_seen DESC \
        LIMIT 10";
    let parsed = Parser::parse(query_str).expect("VelesQL should parse");

    let query_vec = helpers::generate_embedding(40, 128);
    let mut params = HashMap::new();
    params.insert(
        "current_threat_embedding".to_string(),
        embedding_to_json_param(&query_vec),
    );

    let results = collection
        .execute_query(&parsed, &params)
        .expect("Query should execute");

    // Matching (threat_level > 0.8 AND first_seen > 1735689600):
    //   id1(0.95,1738M), id2(0.85,1738.1M), id6(0.88,1738.3M),
    //   id8(0.91,1738.05M), id10(0.89,1738.4M) = 5
    // Excluded: id3(low threat), id4(old), id5(low threat), id7(old), id9(low+old)
    assert_eq!(
        results.len(),
        5,
        "Should return 5 high-threat recent entries, got {}",
        results.len()
    );

    for r in &results {
        let payload = r.point.payload.as_ref().expect("Should have payload");

        let threat_level = payload
            .get("threat_level")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        assert!(
            threat_level > 0.8,
            "Threat level {threat_level} should be > 0.8"
        );

        let first_seen = payload
            .get("first_seen")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);
        assert!(
            first_seen > 1_735_689_600,
            "first_seen {first_seen} should be > 1735689600 (2025-01-01)"
        );
    }

    // ORDER BY similarity() DESC — primary sort
    for window in results.windows(2) {
        assert!(
            window[0].score >= window[1].score,
            "Similarity DESC: {:.4} >= {:.4}",
            window[0].score,
            window[1].score
        );
    }

    assert!(results.len() <= 10, "LIMIT 10 should be respected");
}
