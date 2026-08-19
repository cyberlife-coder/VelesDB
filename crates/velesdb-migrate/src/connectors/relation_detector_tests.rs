use super::*;
use crate::connectors::{FieldInfo, SourceSchema};

fn make_schema(fields: Vec<(&str, &str)>) -> SourceSchema {
    SourceSchema {
        source_type: "test".to_string(),
        collection: "items".to_string(),
        dimension: 4,
        total_count: None,
        fields: fields
            .into_iter()
            .map(|(name, ft)| FieldInfo {
                name: name.to_string(),
                field_type: ft.to_string(),
                indexed: false,
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn test_detect_author_id_relation() {
    let schema = make_schema(vec![("author_id", "integer")]);
    let relations = detect_relations(&schema);
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].from_column, "author_id");
    assert_eq!(relations[0].to_table, "author");
    assert_eq!(relations[0].edge_label, "HAS_AUTHOR");
}

#[test]
fn test_detect_weaviate_crossref() {
    let schema = make_schema(vec![("author", "Author")]);
    let relations = detect_relations(&schema);
    assert_eq!(relations.len(), 1);
    assert_eq!(relations[0].from_column, "author");
    assert_eq!(relations[0].to_table, "author");
    assert_eq!(relations[0].edge_label, "HAS_AUTHOR");
}

#[test]
fn test_no_relations_for_normal_fields() {
    let schema = make_schema(vec![("title", "string"), ("price", "float")]);
    let relations = detect_relations(&schema);
    assert!(relations.is_empty());
}

#[test]
fn test_bare_id_field_not_detected() {
    // "_id" with empty base should be ignored
    let schema = make_schema(vec![("_id", "ObjectId")]);
    let relations = detect_relations(&schema);
    assert!(relations.is_empty());
}
