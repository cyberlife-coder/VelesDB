//! Hero Query E2E test — the defining `VelesDB` query from README (VP-007).
//!
//! Tests the flagship query:
//! ```sql
//! MATCH (doc:Document)-[:AUTHORED_BY]->(author:Person)
//! WHERE similarity(doc.embedding, $question) > 0.8
//!   AND doc.category = 'research'
//! RETURN author.name, author.email, doc.title
//! ORDER BY similarity() DESC
//! LIMIT 5;
//! ```
//!
//! # API mapping
//!
//! - `doc.category = 'research'` → `NodePattern.properties` on start node
//!   (`OpenCypher` inline property syntax `(doc:Document {category: 'research'})`)
//! - `similarity()` → `execute_match_with_similarity` scores on end node (Person)
//! - `RETURN alias.property` → `project_properties` resolves from bindings

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::velesql::Value;
use velesdb_core::DistanceMetric;

use crate::helpers;

/// Builds start-node properties for `category = 'research'`.
fn research_properties() -> HashMap<String, Value> {
    let mut props = HashMap::new();
    props.insert(
        "category".to_string(),
        Value::String("research".to_string()),
    );
    props
}

/// Creates the 6 Document nodes (4 research, 2 non-research).
fn document_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            1,
            helpers::generate_embedding(10, 4),
            json!({"_labels": ["Document"], "title": "Neural Architecture Search Survey", "category": "research"}),
        ),
        (
            2,
            helpers::generate_embedding(11, 4),
            json!({"_labels": ["Document"], "title": "Transformer Optimization Techniques", "category": "research"}),
        ),
        (
            3,
            helpers::generate_embedding(12, 4),
            json!({"_labels": ["Document"], "title": "Attention Mechanisms in NLP", "category": "research"}),
        ),
        (
            4,
            helpers::generate_embedding(13, 4),
            json!({"_labels": ["Document"], "title": "Quantum Computing Basics", "category": "research"}),
        ),
        (
            5,
            helpers::generate_embedding(14, 4),
            json!({"_labels": ["Document"], "title": "Company Annual Report", "category": "business"}),
        ),
        (
            6,
            helpers::generate_embedding(15, 4),
            json!({"_labels": ["Document"], "title": "Marketing Strategy Guide", "category": "marketing"}),
        ),
    ]
}

/// Creates the 3 Person nodes with vectors at controlled distance from question (seed 100).
fn person_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        // Alice: seed 100 → identical to question → cosine ≈ 1.0
        (
            10,
            helpers::generate_embedding(100, 4),
            json!({"_labels": ["Person"], "name": "Dr. Alice Chen", "email": "alice@university.edu"}),
        ),
        // Bob: seed 101 → close to question → high similarity
        (
            11,
            helpers::generate_embedding(101, 4),
            json!({"_labels": ["Person"], "name": "Prof. Bob Smith", "email": "bob@research.org"}),
        ),
        // Carol: seed 999 → distant from question → low similarity
        (
            12,
            helpers::generate_embedding(999, 4),
            json!({"_labels": ["Person"], "name": "Carol Johnson", "email": "carol@company.com"}),
        ),
    ]
}

/// Sets up the hero query scenario:
/// - 6 Document nodes (some research, some not)
/// - 3 Person nodes (with vectors near question for similarity scoring)
/// - `AUTHORED_BY` edges linking documents to persons
fn setup_hero_scenario() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (temp_dir, db) = helpers::setup_test_db();
    let collection = helpers::setup_labeled_collection(&db, "hero_docs", 4, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &document_nodes());
    helpers::insert_labeled_nodes(&collection, &person_nodes());

    // AUTHORED_BY edges: Document -> Person
    helpers::add_edges(
        &collection,
        &[
            (100, 1, 10, "AUTHORED_BY"), // Doc 1 authored by Alice
            (101, 2, 10, "AUTHORED_BY"), // Doc 2 authored by Alice
            (102, 3, 11, "AUTHORED_BY"), // Doc 3 authored by Bob
            (103, 4, 11, "AUTHORED_BY"), // Doc 4 authored by Bob
            (104, 5, 12, "AUTHORED_BY"), // Doc 5 authored by Carol (business)
            (105, 6, 12, "AUTHORED_BY"), // Doc 6 authored by Carol (marketing)
        ],
    );

    (temp_dir, collection)
}

// ============================================================================
// Test 1: Graph traversal + start-node property filter + RETURN projection
// ============================================================================

#[test]
fn test_hero_query_graph_traversal_and_property_filter() {
    let (_dir, collection) = setup_hero_scenario();

    // MATCH (doc:Document {category: 'research'})-[:AUTHORED_BY]->(author:Person)
    // RETURN author.name, author.email, doc.title
    // LIMIT 10
    let match_clause = helpers::build_single_hop_match(
        "doc",
        "Document",
        "AUTHORED_BY",
        "author",
        "Person",
        research_properties(),
        None, // No WHERE needed — category filtered via start node properties
        vec![
            ("author.name", None),
            ("author.email", None),
            ("doc.title", None),
        ],
        None,
        Some(10),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    // Research docs (1,2,3,4) each link to one Person → 4 results expected
    assert_eq!(
        results.len(),
        4,
        "Should return 4 results (one per research doc), got {}",
        results.len()
    );

    // All results should be Person nodes (targets of AUTHORED_BY)
    let author_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    for &id in &author_ids {
        assert!(
            id == 10 || id == 11,
            "Result node_id {id} should be Alice (10) or Bob (11)"
        );
    }

    // Carol (12) should NOT appear — she only authored non-research docs
    assert!(
        !author_ids.contains(&12),
        "Carol (12) should be excluded — her docs are not 'research'"
    );
}

// ============================================================================
// Test 2: Similarity scoring + ordering (execute_match_with_similarity)
// ============================================================================

#[test]
fn test_hero_query_similarity_scoring_and_ordering() {
    let (_dir, collection) = setup_hero_scenario();

    // Similarity is scored on end nodes (Person).
    // Alice (seed 100) ≈ question (seed 100) → highest similarity.
    // Bob (seed 101) → slightly lower.
    // Carol (seed 999) → low, but excluded by property filter anyway.
    let match_clause = helpers::build_single_hop_match(
        "doc",
        "Document",
        "AUTHORED_BY",
        "author",
        "Person",
        research_properties(),
        None,
        vec![
            ("author.name", None),
            ("author.email", None),
            ("doc.title", None),
        ],
        Some(vec![("similarity()", true)]), // ORDER BY similarity() DESC
        Some(5),
    );

    let question_vector = helpers::generate_embedding(100, 4);

    // Threshold 0.0 ensures all graph-matched results pass similarity filter
    let results = collection
        .execute_match_with_similarity(&match_clause, &question_vector, 0.0, &HashMap::new())
        .expect("execute_match_with_similarity failed");

    // 1. Results are non-empty
    assert!(
        !results.is_empty(),
        "Hero query should return at least one result"
    );

    // 2. All results have similarity scores
    for result in &results {
        assert!(
            result.score.is_some(),
            "Each result should have a similarity score"
        );
    }

    // 3. Results are ordered by similarity DESC (highest first)
    let scores: Vec<f32> = results.iter().filter_map(|r| r.score).collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "Results should be ordered by similarity DESC: {:.4} >= {:.4}",
            window[0],
            window[1]
        );
    }

    // 4. Alice-authored results (node_id=10) should have the highest scores
    //    because Alice's vector (seed 100) == question vector (seed 100)
    if let Some(first) = results.first() {
        assert_eq!(
            first.node_id, 10,
            "Highest similarity result should be Alice (10), got {}",
            first.node_id
        );
    }

    // 5. LIMIT 5 respected
    assert!(
        results.len() <= 5,
        "Should return at most 5 results, got {}",
        results.len()
    );
}

// ============================================================================
// Test 3: Property filter excludes non-research documents
// ============================================================================

#[test]
fn test_hero_query_excludes_non_research_docs() {
    let (_dir, collection) = setup_hero_scenario();

    let match_clause = helpers::build_single_hop_match(
        "doc",
        "Document",
        "AUTHORED_BY",
        "author",
        "Person",
        research_properties(),
        None,
        vec![("doc.title", None), ("author.name", None)],
        None,
        Some(100),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    // Should only have results from research docs (1, 2, 3, 4)
    // which are authored by Alice (10) and Bob (11) — NOT Carol (12)
    assert_eq!(
        results.len(),
        4,
        "Should have exactly 4 results (research docs only), got {}",
        results.len()
    );

    let author_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(
        !author_ids.contains(&12),
        "Carol (12) should not appear in results"
    );
}

// ============================================================================
// Test 4: Projected properties from BOTH doc and author nodes
// ============================================================================

#[test]
fn test_hero_query_projected_properties_from_both_nodes() {
    let (_dir, collection) = setup_hero_scenario();

    let match_clause = helpers::build_single_hop_match(
        "doc",
        "Document",
        "AUTHORED_BY",
        "author",
        "Person",
        research_properties(),
        None,
        vec![
            ("author.name", None),
            ("author.email", None),
            ("doc.title", None),
        ],
        None,
        Some(5),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    assert!(!results.is_empty(), "Should have results");

    // Check that at least one result has both author and doc properties projected
    let has_both = results
        .iter()
        .any(|r| r.projected.contains_key("author.name") && r.projected.contains_key("doc.title"));

    assert!(
        has_both,
        "At least one result should have both author.name and doc.title projected. \
         First result projected: {:?}",
        results.first().map(|r| &r.projected)
    );

    // Verify actual property values are strings (not null)
    for result in &results {
        if let Some(name) = result.projected.get("author.name") {
            assert!(
                name.is_string(),
                "author.name should be a string, got: {name}"
            );
        }
        if let Some(email) = result.projected.get("author.email") {
            assert!(
                email.is_string(),
                "author.email should be a string, got: {email}"
            );
        }
        if let Some(title) = result.projected.get("doc.title") {
            assert!(
                title.is_string(),
                "doc.title should be a string, got: {title}"
            );
        }
    }
}
