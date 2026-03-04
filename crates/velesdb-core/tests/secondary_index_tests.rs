#![cfg(feature = "persistence")]

use std::collections::HashSet;

use serde_json::json;
use tempfile::tempdir;
use velesdb_core::velesql::{IndexType, Parser, PlanNode, QueryPlan};
use velesdb_core::{DistanceMetric, Point, Result, StorageMode, VectorCollection};

#[test]
fn secondary_index_accelerates_metadata_query_and_explain() -> Result<()> {
    let dir = tempdir()?;
    let collection = VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )?;
    collection.create_index("category")?;

    collection.upsert(vec![
        Point::new(
            1,
            vec![1.0, 0.0],
            Some(json!({"category": "books", "title": "A"})),
        ),
        Point::new(
            2,
            vec![0.0, 1.0],
            Some(json!({"category": "music", "title": "B"})),
        ),
        Point::new(
            3,
            vec![0.5, 0.5],
            Some(json!({"category": "books", "title": "C"})),
        ),
    ])?;

    let query = Parser::parse("SELECT * FROM docs WHERE category = 'books' LIMIT 10")?;
    let mut indexed_fields = HashSet::new();
    indexed_fields.insert("category".to_string());
    let plan = QueryPlan::from_select_with_indexed_fields(&query.select, &indexed_fields);

    assert_eq!(plan.index_used, Some(IndexType::Property));
    assert!(matches!(plan.root, PlanNode::Sequence(_)));

    let results = collection.execute_query(&query, &std::collections::HashMap::new())?;
    let ids: Vec<u64> = results.into_iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![1, 3]);

    Ok(())
}

#[test]
fn secondary_index_is_updated_on_delete() -> Result<()> {
    let dir = tempdir()?;
    let collection = VectorCollection::create(
        dir.path().join("docs"),
        "docs",
        2,
        DistanceMetric::Cosine,
        StorageMode::Full,
    )?;
    collection.create_index("category")?;

    collection.upsert(vec![
        Point::new(10, vec![1.0, 0.0], Some(json!({"category": "books"}))),
        Point::new(11, vec![0.0, 1.0], Some(json!({"category": "books"}))),
    ])?;

    collection.delete(&[10])?;

    let query = Parser::parse("SELECT * FROM docs WHERE category = 'books' LIMIT 10")?;
    let results = collection.execute_query(&query, &std::collections::HashMap::new())?;
    let ids: Vec<u64> = results.into_iter().map(|r| r.point.id).collect();
    assert_eq!(ids, vec![11]);

    Ok(())
}
