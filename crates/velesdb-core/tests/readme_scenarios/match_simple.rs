//! Simple MATCH business scenario tests (BS1: E-commerce, BS4: Agent Memory).
//!
//! Tests created by Phase 4 Plan 04-04 (VP-007).
//!
//! - **BS1**: Single-hop `(product:Product)-[:SUPPLIED_BY]->(supplier:Supplier)`
//!   with similarity threshold + supplier trust_score filter.
//! - **BS4**: Multi-hop `(user:User)-[:HAD_CONVERSATION]->(conv)-[:CONTAINS]->(msg)`
//!   with binding-aware temporal filter on intermediate node.

use std::collections::HashMap;

use serde_json::json;

use velesdb_core::velesql::{
    CompareOp, Comparison, Condition, Direction, GraphPattern, NodePattern, RelationshipPattern,
    Value,
};
use velesdb_core::DistanceMetric;

use crate::helpers;

// ============================================================================
// BS1: E-commerce Product Discovery
// ============================================================================
//
// README query (simplified — subquery tested in Plan 04-06):
// ```sql
// MATCH (product:Product)-[:SUPPLIED_BY]->(supplier:Supplier)
// WHERE
//   similarity(product.image_embedding, $uploaded_photo) > 0.7
//   AND supplier.trust_score > 4.5
// ORDER BY similarity() DESC
// LIMIT 12
// ```

/// 8 Products: 6 with high-similarity vectors (seed near query), 2 distant.
fn product_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            1,
            helpers::generate_embedding(100, 4), // identical to query → sim ≈ 1.0
            json!({"_labels": ["Product"], "name": "Running Shoes Pro", "price": 120.0, "sku": "SKU001"}),
        ),
        (
            2,
            helpers::generate_embedding(101, 4), // close to query → high sim
            json!({"_labels": ["Product"], "name": "Trail Runners Elite", "price": 150.0, "sku": "SKU002"}),
        ),
        (
            3,
            helpers::generate_embedding(102, 4), // close
            json!({"_labels": ["Product"], "name": "Sprint Sneakers", "price": 89.0, "sku": "SKU003"}),
        ),
        (
            4,
            helpers::generate_embedding(103, 4), // close
            json!({"_labels": ["Product"], "name": "Urban Walker", "price": 200.0, "sku": "SKU004"}),
        ),
        (
            5,
            helpers::generate_embedding(500, 4), // distant → low similarity
            json!({"_labels": ["Product"], "name": "Formal Oxfords", "price": 300.0, "sku": "SKU005"}),
        ),
        (
            6,
            helpers::generate_embedding(501, 4), // distant → low similarity
            json!({"_labels": ["Product"], "name": "Dress Loafers", "price": 250.0, "sku": "SKU006"}),
        ),
    ]
}

/// 3 Suppliers: 2 high-trust (>4.5), 1 low-trust.
fn supplier_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            10,
            helpers::generate_embedding(200, 4),
            json!({"_labels": ["Supplier"], "name": "TrustyCo", "trust_score": 4.8}),
        ),
        (
            11,
            helpers::generate_embedding(201, 4),
            json!({"_labels": ["Supplier"], "name": "ReliableInc", "trust_score": 4.9}),
        ),
        (
            12,
            helpers::generate_embedding(202, 4),
            json!({"_labels": ["Supplier"], "name": "ShadySupplies", "trust_score": 3.2}),
        ),
    ]
}

/// Sets up BS1 scenario: Products, Suppliers, and SUPPLIED_BY edges.
fn setup_bs1_scenario() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (temp_dir, db) = helpers::setup_test_db();
    let collection = helpers::setup_labeled_collection(&db, "ecommerce", 4, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &product_nodes());
    helpers::insert_labeled_nodes(&collection, &supplier_nodes());

    // SUPPLIED_BY edges: Product → Supplier
    helpers::add_edges(
        &collection,
        &[
            (100, 1, 10, "SUPPLIED_BY"), // Running Shoes → TrustyCo (high trust)
            (101, 2, 10, "SUPPLIED_BY"), // Trail Runners → TrustyCo (high trust)
            (102, 3, 11, "SUPPLIED_BY"), // Sprint Sneakers → ReliableInc (high trust)
            (103, 4, 12, "SUPPLIED_BY"), // Urban Walker → ShadySupplies (LOW trust)
            (104, 5, 12, "SUPPLIED_BY"), // Formal Oxfords → ShadySupplies (LOW, distant)
            (105, 6, 11, "SUPPLIED_BY"), // Dress Loafers → ReliableInc (high, distant)
        ],
    );

    (temp_dir, collection)
}

/// BS1 core test: single-hop MATCH + similarity + supplier trust_score filter.
///
/// Verifies:
/// - MATCH traversal follows SUPPLIED_BY edges correctly
/// - Similarity threshold filters low-similarity products
/// - Supplier trust_score filter works via WHERE on target node
/// - Results ordered by similarity DESC
#[test]
fn test_bs1_ecommerce_discovery() {
    let (_dir, collection) = setup_bs1_scenario();

    // WHERE: trust_score > 4.5 (evaluated on target = Supplier node)
    let where_clause = Condition::Comparison(Comparison {
        column: "trust_score".to_string(),
        operator: CompareOp::Gt,
        value: Value::Float(4.5),
    });

    let match_clause = helpers::build_single_hop_match(
        "product",
        "Product",
        "SUPPLIED_BY",
        "supplier",
        "Supplier",
        HashMap::new(), // No start-node property filter
        Some(where_clause),
        vec![
            ("product.name", None),
            ("supplier.name", None),
            ("supplier.trust_score", None),
        ],
        Some(vec![("similarity()", true)]), // ORDER BY similarity() DESC
        Some(12),
    );

    let query_vector = helpers::generate_embedding(100, 4);

    // Similarity threshold 0.5 → filters distant products (seeds 500, 501)
    let results = collection
        .execute_match_with_similarity(&match_clause, &query_vector, 0.5, &HashMap::new())
        .expect("execute_match_with_similarity failed");

    // Products 1,2,3 → high-trust suppliers (TrustyCo, ReliableInc)
    // Product 4 → ShadySupplies (trust 3.2) → filtered by WHERE
    // Products 5,6 → distant vectors → filtered by similarity threshold
    assert!(
        !results.is_empty(),
        "BS1 should return results for high-trust, high-similarity products"
    );

    // All results should come from high-trust suppliers (node 10 or 11)
    for result in &results {
        assert!(
            result.node_id == 10 || result.node_id == 11,
            "Result supplier {} should be TrustyCo (10) or ReliableInc (11)",
            result.node_id
        );
    }

    // ShadySupplies (12) must NOT appear (trust_score = 3.2 < 4.5)
    let supplier_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    assert!(
        !supplier_ids.contains(&12),
        "ShadySupplies (12) should be excluded by trust_score filter"
    );

    // Results should be ordered by similarity DESC
    let scores: Vec<f32> = results.iter().filter_map(|r| r.score).collect();
    for window in scores.windows(2) {
        assert!(
            window[0] >= window[1],
            "Results should be ordered DESC: {:.4} >= {:.4}",
            window[0],
            window[1]
        );
    }
}

/// BS1 projected properties: verifies cross-node property projection.
#[test]
fn test_bs1_cross_node_projection() {
    let (_dir, collection) = setup_bs1_scenario();

    let match_clause = helpers::build_single_hop_match(
        "product",
        "Product",
        "SUPPLIED_BY",
        "supplier",
        "Supplier",
        HashMap::new(),
        None, // No WHERE — test projection only
        vec![
            ("product.name", None),
            ("product.price", None),
            ("supplier.name", None),
            ("supplier.trust_score", None),
        ],
        None,
        Some(20),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    // All 6 products have SUPPLIED_BY edges → 6 results
    assert_eq!(
        results.len(),
        6,
        "Should return 6 results (one per product), got {}",
        results.len()
    );

    // At least one result should have both product and supplier properties projected
    let has_both = results.iter().any(|r| {
        r.projected.contains_key("product.name") && r.projected.contains_key("supplier.name")
    });
    assert!(
        has_both,
        "Results should have cross-node projected properties (product.name + supplier.name). \
         First result projected: {:?}",
        results.first().map(|r| &r.projected)
    );

    // Verify supplier.trust_score is a number (not null)
    for result in &results {
        if let Some(ts) = result.projected.get("supplier.trust_score") {
            assert!(
                ts.is_number(),
                "supplier.trust_score should be a number, got: {}",
                ts
            );
        }
    }
}

/// BS1 similarity threshold: ensures low-similarity products are filtered.
#[test]
fn test_bs1_similarity_threshold_filters() {
    let (_dir, collection) = setup_bs1_scenario();

    let match_clause = helpers::build_single_hop_match(
        "product",
        "Product",
        "SUPPLIED_BY",
        "supplier",
        "Supplier",
        HashMap::new(),
        None, // No WHERE — test similarity filtering only
        vec![("product.name", None), ("supplier.name", None)],
        Some(vec![("similarity()", true)]),
        Some(12),
    );

    let query_vector = helpers::generate_embedding(100, 4);

    // High threshold (0.95) → only the most similar product(s) pass
    let strict_results = collection
        .execute_match_with_similarity(&match_clause, &query_vector, 0.95, &HashMap::new())
        .expect("strict threshold query failed");

    // Low threshold (0.0) → all products pass
    let loose_results = collection
        .execute_match_with_similarity(&match_clause, &query_vector, 0.0, &HashMap::new())
        .expect("loose threshold query failed");

    assert!(
        strict_results.len() <= loose_results.len(),
        "Strict threshold ({}) should return fewer or equal results than loose ({})",
        strict_results.len(),
        loose_results.len()
    );

    // Strict should still have results (product 1 has identical vector to query)
    assert!(
        !strict_results.is_empty(),
        "At least product 1 (identical vector) should pass 0.95 threshold"
    );
}

// ============================================================================
// BS4: AI Agent Memory
// ============================================================================
//
// README query (simplified — temporal as fixed epoch):
// ```sql
// MATCH (user:User)-[:HAD_CONVERSATION]->(conv:Conversation)
//       -[:CONTAINS]->(message:Message)
// WHERE conv.timestamp > 1700000000
// RETURN user.name, conv.timestamp, message.content
// LIMIT 10
// ```

/// 2 Users.
fn user_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            20,
            helpers::generate_embedding(300, 4),
            json!({"_labels": ["User"], "name": "Alice"}),
        ),
        (
            21,
            helpers::generate_embedding(301, 4),
            json!({"_labels": ["User"], "name": "Bob"}),
        ),
    ]
}

/// 4 Conversations: 2 recent (high timestamp), 2 old (low timestamp).
fn conversation_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            30,
            helpers::generate_embedding(400, 4),
            json!({"_labels": ["Conversation"], "timestamp": 1_700_100_000}), // recent
        ),
        (
            31,
            helpers::generate_embedding(401, 4),
            json!({"_labels": ["Conversation"], "timestamp": 1_700_200_000}), // recent
        ),
        (
            32,
            helpers::generate_embedding(402, 4),
            json!({"_labels": ["Conversation"], "timestamp": 1_600_000_000}), // old
        ),
        (
            33,
            helpers::generate_embedding(403, 4),
            json!({"_labels": ["Conversation"], "timestamp": 1_500_000_000}), // old
        ),
    ]
}

/// 6 Messages across conversations.
fn message_nodes() -> Vec<(u64, Vec<f32>, serde_json::Value)> {
    vec![
        (
            40,
            helpers::generate_embedding(500, 4),
            json!({"_labels": ["Message"], "content": "How do I deploy to k8s?"}),
        ),
        (
            41,
            helpers::generate_embedding(501, 4),
            json!({"_labels": ["Message"], "content": "Use kubectl apply -f manifest.yaml"}),
        ),
        (
            42,
            helpers::generate_embedding(502, 4),
            json!({"_labels": ["Message"], "content": "What is HNSW indexing?"}),
        ),
        (
            43,
            helpers::generate_embedding(503, 4),
            json!({"_labels": ["Message"], "content": "It is graph-based approximate NN search"}),
        ),
        (
            44,
            helpers::generate_embedding(504, 4),
            json!({"_labels": ["Message"], "content": "Old topic about weather"}),
        ),
        (
            45,
            helpers::generate_embedding(505, 4),
            json!({"_labels": ["Message"], "content": "Old topic about food"}),
        ),
    ]
}

/// Sets up BS4 scenario: Users → Conversations → Messages.
fn setup_bs4_scenario() -> (tempfile::TempDir, velesdb_core::Collection) {
    let (temp_dir, db) = helpers::setup_test_db();
    let collection =
        helpers::setup_labeled_collection(&db, "agent_memory", 4, DistanceMetric::Cosine);

    helpers::insert_labeled_nodes(&collection, &user_nodes());
    helpers::insert_labeled_nodes(&collection, &conversation_nodes());
    helpers::insert_labeled_nodes(&collection, &message_nodes());

    // HAD_CONVERSATION edges: User → Conversation
    helpers::add_edges(
        &collection,
        &[
            (200, 20, 30, "HAD_CONVERSATION"), // Alice → Conv 30 (recent)
            (201, 20, 32, "HAD_CONVERSATION"), // Alice → Conv 32 (old)
            (202, 21, 31, "HAD_CONVERSATION"), // Bob → Conv 31 (recent)
            (203, 21, 33, "HAD_CONVERSATION"), // Bob → Conv 33 (old)
        ],
    );

    // CONTAINS edges: Conversation → Message
    helpers::add_edges(
        &collection,
        &[
            (300, 30, 40, "CONTAINS"), // Conv 30 (recent) → Msg 40
            (301, 30, 41, "CONTAINS"), // Conv 30 (recent) → Msg 41
            (302, 31, 42, "CONTAINS"), // Conv 31 (recent) → Msg 42
            (303, 31, 43, "CONTAINS"), // Conv 31 (recent) → Msg 43
            (304, 32, 44, "CONTAINS"), // Conv 32 (old) → Msg 44
            (305, 33, 45, "CONTAINS"), // Conv 33 (old) → Msg 45
        ],
    );

    (temp_dir, collection)
}

/// Builds the multi-hop pattern for BS4:
/// `(user:User)-[:HAD_CONVERSATION]->(conv:Conversation)-[:CONTAINS]->(message:Message)`
fn build_bs4_pattern() -> GraphPattern {
    GraphPattern {
        name: None,
        nodes: vec![
            NodePattern::new().with_alias("user").with_label("User"),
            NodePattern::new()
                .with_alias("conv")
                .with_label("Conversation"),
            NodePattern::new()
                .with_alias("message")
                .with_label("Message"),
        ],
        relationships: vec![
            RelationshipPattern {
                alias: None,
                types: vec!["HAD_CONVERSATION".to_string()],
                direction: Direction::Outgoing,
                range: None,
                properties: HashMap::new(),
            },
            RelationshipPattern {
                alias: None,
                types: vec!["CONTAINS".to_string()],
                direction: Direction::Outgoing,
                range: None,
                properties: HashMap::new(),
            },
        ],
    }
}

/// BS4 core test: multi-hop MATCH (2 hops) + temporal filter on intermediate node.
///
/// Verifies:
/// - Multi-hop traversal reaches Message nodes through Conversations
/// - Binding-aware WHERE filters on intermediate node (conv.timestamp)
/// - All 3 node aliases populated in bindings
/// - Cross-node property projection works
#[test]
fn test_bs4_agent_memory() {
    let (_dir, collection) = setup_bs4_scenario();

    // WHERE: conv.timestamp > 1700000000 (binding-aware, alias-qualified)
    let where_clause = Condition::Comparison(Comparison {
        column: "conv.timestamp".to_string(),
        operator: CompareOp::Gt,
        value: Value::Integer(1_700_000_000),
    });

    let match_clause = helpers::build_match_clause(
        vec![build_bs4_pattern()],
        Some(where_clause),
        vec![
            ("user.name", None),
            ("conv.timestamp", None),
            ("message.content", None),
        ],
        Some(vec![("conv.timestamp", true)]), // ORDER BY conv.timestamp DESC (VP-006 fixed)
        Some(10),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed for BS4");

    // Recent conversations: Conv 30 (ts=1700100000) has 2 messages, Conv 31 (ts=1700200000) has 2
    // Old conversations: Conv 32, 33 filtered by timestamp
    // Expected: 4 message results (from Conv 30 + Conv 31)
    assert_eq!(
        results.len(),
        4,
        "Should return 4 messages from recent conversations, got {}",
        results.len()
    );

    // All results should be Message nodes (final hop targets)
    let message_ids: Vec<u64> = results.iter().map(|r| r.node_id).collect();
    for &id in &message_ids {
        assert!(
            (40..=43).contains(&id),
            "Result node {} should be a recent message (40-43)",
            id
        );
    }

    // Old messages (44, 45) must NOT appear
    assert!(
        !message_ids.contains(&44) && !message_ids.contains(&45),
        "Old messages (44, 45) should be excluded by timestamp filter"
    );
}

/// BS4 binding verification: all 3 aliases present in bindings.
#[test]
fn test_bs4_bindings_all_aliases_populated() {
    let (_dir, collection) = setup_bs4_scenario();

    let match_clause = helpers::build_match_clause(
        vec![build_bs4_pattern()],
        None, // No WHERE — test bindings only
        vec![
            ("user.name", None),
            ("conv.timestamp", None),
            ("message.content", None),
        ],
        None,
        Some(20),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    // Total: 6 messages across all conversations
    assert_eq!(
        results.len(),
        6,
        "Should return all 6 messages (no filter), got {}",
        results.len()
    );

    // Every result must have all 3 aliases in bindings
    for result in &results {
        assert!(
            result.bindings.contains_key("user"),
            "Result for node {} missing 'user' binding. Bindings: {:?}",
            result.node_id,
            result.bindings
        );
        assert!(
            result.bindings.contains_key("conv"),
            "Result for node {} missing 'conv' binding. Bindings: {:?}",
            result.node_id,
            result.bindings
        );
        assert!(
            result.bindings.contains_key("message"),
            "Result for node {} missing 'message' binding. Bindings: {:?}",
            result.node_id,
            result.bindings
        );
    }
}

/// BS4 cross-node projection: properties from all 3 node types.
#[test]
fn test_bs4_cross_node_projection() {
    let (_dir, collection) = setup_bs4_scenario();

    let match_clause = helpers::build_match_clause(
        vec![build_bs4_pattern()],
        None,
        vec![
            ("user.name", None),
            ("conv.timestamp", None),
            ("message.content", None),
        ],
        None,
        Some(20),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    assert!(!results.is_empty(), "Should have results");

    // At least one result should have properties from all 3 nodes
    let has_all_three = results.iter().any(|r| {
        r.projected.contains_key("user.name")
            && r.projected.contains_key("conv.timestamp")
            && r.projected.contains_key("message.content")
    });
    assert!(
        has_all_three,
        "At least one result should project properties from user, conv, and message. \
         First result projected: {:?}",
        results.first().map(|r| &r.projected)
    );

    // Verify value types
    for result in &results {
        if let Some(name) = result.projected.get("user.name") {
            assert!(
                name.is_string(),
                "user.name should be a string, got: {}",
                name
            );
        }
        if let Some(ts) = result.projected.get("conv.timestamp") {
            assert!(
                ts.is_number(),
                "conv.timestamp should be a number, got: {}",
                ts
            );
        }
        if let Some(content) = result.projected.get("message.content") {
            assert!(
                content.is_string(),
                "message.content should be a string, got: {}",
                content
            );
        }
    }
}

/// BS4 ORDER BY property test (VP-006): verifies results are sorted by conv.timestamp DESC.
///
/// Uses the same multi-hop pattern but focuses on verifying that ORDER BY
/// on an intermediate node's projected property actually reorders results.
#[test]
fn test_bs4_order_by_timestamp() {
    let (_dir, collection) = setup_bs4_scenario();

    // No WHERE filter — return all 6 messages, but ORDER BY conv.timestamp DESC
    let match_clause = helpers::build_match_clause(
        vec![build_bs4_pattern()],
        None,
        vec![
            ("user.name", None),
            ("conv.timestamp", None),
            ("message.content", None),
        ],
        Some(vec![("conv.timestamp", true)]), // ORDER BY conv.timestamp DESC
        Some(20),
    );

    let results = collection
        .execute_match(&match_clause, &HashMap::new())
        .expect("execute_match failed");

    assert_eq!(
        results.len(),
        6,
        "Should return all 6 messages (no filter), got {}",
        results.len()
    );

    // Extract conv.timestamp from projected properties for each result
    let timestamps: Vec<Option<i64>> = results
        .iter()
        .map(|r| r.projected.get("conv.timestamp").and_then(|v| v.as_i64()))
        .collect();

    // All results should have a projected conv.timestamp
    for (i, ts) in timestamps.iter().enumerate() {
        assert!(
            ts.is_some(),
            "Result {} (node {}) should have conv.timestamp projected, got None. Projected: {:?}",
            i,
            results[i].node_id,
            results[i].projected
        );
    }

    // Verify DESC ordering: each timestamp >= the next
    let ts_values: Vec<i64> = timestamps.iter().filter_map(|t| *t).collect();
    for window in ts_values.windows(2) {
        assert!(
            window[0] >= window[1],
            "Results should be ordered by conv.timestamp DESC: {} >= {}",
            window[0],
            window[1]
        );
    }

    // Conv 31 (ts=1700200000) messages should appear before Conv 30 (ts=1700100000) messages
    // Conv 30 before Conv 32 (ts=1600000000) before Conv 33 (ts=1500000000)
    assert_eq!(
        ts_values.first().copied(),
        Some(1_700_200_000),
        "First result should be from most recent conversation (1700200000)"
    );
    assert_eq!(
        ts_values.last().copied(),
        Some(1_500_000_000),
        "Last result should be from oldest conversation (1500000000)"
    );
}
