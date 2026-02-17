use std::collections::HashMap;

use serde_json::json;
use tempfile::TempDir;
use velesdb_core::{
    velesql::{
        Direction, GraphPattern, MatchClause, NodePattern, RelationshipPattern, ReturnClause,
        ReturnItem,
    },
    Collection, Database, DistanceMetric, Error, GraphEdge, Point,
};

const DIM: usize = 4;

fn setup_collection_with_graph() -> (TempDir, Collection) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open db");
    db.create_collection("docs", DIM, DistanceMetric::Cosine)
        .expect("create collection");
    let collection = db.get_collection("docs").expect("get collection");

    collection
        .upsert(vec![
            Point::new(
                1,
                vec![1.0, 0.0, 0.0, 0.0],
                Some(json!({"label": "Article"})),
            ),
            Point::new(
                2,
                vec![0.9, 0.1, 0.0, 0.0],
                Some(json!({"label": "Article"})),
            ),
        ])
        .expect("upsert points");

    let edge = GraphEdge::new(100, 1, 2, "RELATED_TO").expect("create edge");
    collection.add_edge(edge).expect("add edge");

    (dir, collection)
}

fn build_match_clause() -> MatchClause {
    let pattern = GraphPattern {
        name: None,
        nodes: vec![
            NodePattern::new().with_alias("a"),
            NodePattern::new().with_alias("b"),
        ],
        relationships: vec![RelationshipPattern {
            alias: None,
            types: vec!["RELATED_TO".to_string()],
            direction: Direction::Outgoing,
            range: None,
            properties: HashMap::new(),
        }],
    };

    MatchClause {
        patterns: vec![pattern],
        where_clause: None,
        return_clause: ReturnClause {
            items: vec![ReturnItem {
                expression: "*".to_string(),
                alias: None,
            }],
            order_by: None,
            limit: Some(10),
        },
    }
}

#[test]
fn test_execute_match_with_similarity_dimension_mismatch_returns_error() {
    let (_dir, collection) = setup_collection_with_graph();
    let match_clause = build_match_clause();
    let wrong_dim_query = vec![1.0, 0.0];

    let err = collection
        .execute_match_with_similarity(&match_clause, &wrong_dim_query, 0.0, &HashMap::new())
        .expect_err("dimension mismatch must return an error");

    match err {
        Error::DimensionMismatch { expected, actual } => {
            assert_eq!(expected, DIM);
            assert_eq!(actual, wrong_dim_query.len());
        }
        other => panic!("expected DimensionMismatch, got {other:?}"),
    }
}
