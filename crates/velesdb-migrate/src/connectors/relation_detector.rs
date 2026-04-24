//! Auto-detection of source relations for graph migration.
//!
//! Analyses the source schema to infer FK-like relationships
//! using naming conventions and source-specific signals.

use crate::config::RelationConfig;
use crate::connectors::SourceSchema;

/// Detects probable relations from a source schema using naming heuristics.
///
/// Rules:
/// - A field whose name ends with `_id` is treated as a FK to another table.
///   The target table name is inferred by removing the `_id` suffix.
///   Edge label is `HAS_<UPPERCASED_BASE>`.
///
/// - For Weaviate: a field whose type starts with an uppercase letter
///   (cross-reference to a class) and is fully alphanumeric is detected
///   as a relation.
#[must_use]
pub fn detect_relations(schema: &SourceSchema) -> Vec<RelationConfig> {
    schema
        .fields
        .iter()
        .filter_map(detect_single_relation)
        .collect()
}

fn detect_single_relation(field: &crate::connectors::FieldInfo) -> Option<RelationConfig> {
    // Strategy 1: column name ends with _id (common FK convention)
    if let Some(base) = field.name.strip_suffix("_id") {
        if base.is_empty() {
            return None;
        }
        let edge_label = format!("HAS_{}", base.to_uppercase());
        return Some(RelationConfig {
            from_column: field.name.clone(),
            to_table: base.to_string(),
            to_column: "id".to_string(),
            edge_label,
            weight_column: None,
        });
    }

    // Strategy 2: Weaviate cross-reference -- field_type starts with uppercase
    // (Weaviate class names are PascalCase, e.g., "Author", "Category")
    if field
        .field_type
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase())
        && !field.field_type.is_empty()
        && field.field_type.chars().all(|c| c.is_alphanumeric())
    {
        let edge_label = format!("HAS_{}", field.name.to_uppercase());
        return Some(RelationConfig {
            from_column: field.name.clone(),
            to_table: field.field_type.to_lowercase(),
            to_column: "id".to_string(),
            edge_label,
            weight_column: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
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
}
