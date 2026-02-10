//! Shared test utilities for README scenario tests (VP-007).
//!
//! Provides helpers for DB setup, deterministic vector generation,
//! graph construction, and query execution.

use std::collections::HashMap;

use tempfile::TempDir;

use velesdb_core::collection::graph::GraphEdge;
use velesdb_core::velesql::{
    Direction, GraphPattern, MatchClause, NodePattern, OrderByItem, RelationshipPattern,
    ReturnClause, ReturnItem,
};
use velesdb_core::{Collection, Database, DistanceMetric, Point};

/// Creates a temporary database for test isolation.
///
/// Returns `(TempDir, Database)` — keep `TempDir` alive for the test duration.
pub fn setup_test_db() -> (TempDir, Database) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let db = Database::open(temp_dir.path()).expect("Failed to open database");
    (temp_dir, db)
}

/// Generates a deterministic embedding vector from a seed.
///
/// Produces normalized-ish vectors suitable for cosine similarity testing.
/// Different seeds yield different directions; same seed always yields same vector.
#[allow(clippy::cast_precision_loss)] // Reason: seed/dim values are small test constants
pub fn generate_embedding(seed: u64, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim)
        .map(|i| (seed as f32 * 1.618_034 + i as f32 * 0.577_215_7).sin())
        .collect();

    // Normalize so cosine similarity is meaningful
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Creates a collection with the given name, dimension, and metric.
pub fn setup_labeled_collection(
    db: &Database,
    name: &str,
    dim: usize,
    metric: DistanceMetric,
) -> Collection {
    db.create_collection(name, dim, metric)
        .expect("Failed to create collection");
    db.get_collection(name).expect("Failed to get collection")
}

/// Bulk-inserts nodes with vectors and JSON payloads.
///
/// Each tuple is `(id, vector, payload_json)`.
pub fn insert_labeled_nodes(collection: &Collection, nodes: &[(u64, Vec<f32>, serde_json::Value)]) {
    let points: Vec<Point> = nodes
        .iter()
        .map(|(id, vec, payload)| Point::new(*id, vec.clone(), Some(payload.clone())))
        .collect();
    collection.upsert(points).expect("Failed to upsert points");
}

/// Bulk-adds graph edges.
///
/// Each tuple is `(edge_id, source_id, target_id, label)`.
pub fn add_edges(collection: &Collection, edges: &[(u64, u64, u64, &str)]) {
    for &(edge_id, source, target, label) in edges {
        let edge =
            GraphEdge::new(edge_id, source, target, label).expect("Failed to create GraphEdge");
        collection.add_edge(edge).expect("Failed to add edge");
    }
}

/// Builds a single-hop `MatchClause` programmatically.
///
/// Pattern: `(start_alias:start_label {start_properties})-[:rel_type]->(end_alias:end_label)`
///
/// `start_properties` filters start nodes via inline property matching (OpenCypher style).
/// Use this instead of WHERE for properties on the start node, since single-hop WHERE
/// evaluates on the target (end) node.
#[allow(clippy::too_many_arguments)] // Reason: mirrors MatchClause AST structure directly
pub fn build_single_hop_match(
    start_alias: &str,
    start_label: &str,
    rel_type: &str,
    end_alias: &str,
    end_label: &str,
    start_properties: HashMap<String, velesdb_core::velesql::Value>,
    where_clause: Option<velesdb_core::velesql::Condition>,
    return_items: Vec<(&str, Option<&str>)>,
    order_by: Option<Vec<(&str, bool)>>,
    limit: Option<u64>,
) -> MatchClause {
    let mut start_node = NodePattern::new()
        .with_alias(start_alias)
        .with_label(start_label);
    start_node.properties = start_properties;

    let pattern = GraphPattern {
        name: None,
        nodes: vec![
            start_node,
            NodePattern::new()
                .with_alias(end_alias)
                .with_label(end_label),
        ],
        relationships: vec![RelationshipPattern {
            alias: None,
            types: vec![rel_type.to_string()],
            direction: Direction::Outgoing,
            range: None,
            properties: HashMap::new(),
        }],
    };

    let items: Vec<ReturnItem> = return_items
        .into_iter()
        .map(|(expr, alias)| ReturnItem {
            expression: expr.to_string(),
            alias: alias.map(str::to_string),
        })
        .collect();

    let order = order_by.map(|obs| {
        obs.into_iter()
            .map(|(expr, desc)| OrderByItem {
                expression: expr.to_string(),
                descending: desc,
            })
            .collect()
    });

    MatchClause {
        patterns: vec![pattern],
        where_clause,
        return_clause: ReturnClause {
            items,
            order_by: order,
            limit,
        },
    }
}

/// Builds a `MatchClause` with arbitrary patterns (for multi-hop or custom structures).
pub fn build_match_clause(
    patterns: Vec<GraphPattern>,
    where_clause: Option<velesdb_core::velesql::Condition>,
    return_items: Vec<(&str, Option<&str>)>,
    order_by: Option<Vec<(&str, bool)>>,
    limit: Option<u64>,
) -> MatchClause {
    let items: Vec<ReturnItem> = return_items
        .into_iter()
        .map(|(expr, alias)| ReturnItem {
            expression: expr.to_string(),
            alias: alias.map(str::to_string),
        })
        .collect();

    let order = order_by.map(|obs| {
        obs.into_iter()
            .map(|(expr, desc)| OrderByItem {
                expression: expr.to_string(),
                descending: desc,
            })
            .collect()
    });

    MatchClause {
        patterns,
        where_clause,
        return_clause: ReturnClause {
            items,
            order_by: order,
            limit,
        },
    }
}
