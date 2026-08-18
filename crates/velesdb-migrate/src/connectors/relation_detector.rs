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
#[path = "relation_detector_tests.rs"]
mod tests;
