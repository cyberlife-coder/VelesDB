//! Private helpers for DML edge and node statement parsing.
//!
//! Extracted from `dml.rs` to keep that module focused on the top-level
//! `Parser` impl methods for each DML statement type.

use super::{extract_identifier, Rule};
use crate::velesql::ast::{
    DmlStatement, InsertEdgeStatement, InsertNodeStatement, Query, Value,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

/// Validates multi-row INSERT/UPSERT: non-empty columns, at least one row,
/// and every row length matches the column count.
pub(super) fn validate_insert_rows(
    columns: &[String],
    rows: &[Vec<Value>],
    context: &str,
) -> Result<(), ParseError> {
    if columns.is_empty() {
        return Err(ParseError::syntax(
            0,
            "",
            format!("{context} requires at least one target column"),
        ));
    }
    if rows.is_empty() {
        return Err(ParseError::syntax(
            0,
            "",
            format!("{context} requires at least one VALUES row"),
        ));
    }
    for row in rows {
        if row.len() != columns.len() {
            return Err(ParseError::syntax(
                0,
                "",
                format!("{context} columns/value count mismatch"),
            ));
        }
    }
    Ok(())
}

/// Extracts collection name and WHERE clause from a DELETE pair.
pub(super) fn extract_delete_fields(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(String, crate::velesql::ast::Condition), ParseError> {
    let mut table: Option<String> = None;
    let mut where_clause = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier => table = Some(extract_identifier(&inner)),
            Rule::where_clause => where_clause = Some(Parser::parse_where_clause(inner)?),
            _ => {}
        }
    }

    Ok((
        require_field(table, "DELETE", "a target collection")?,
        require_field(where_clause, "DELETE", "a WHERE clause")?,
    ))
}

/// Extracts edge ID and collection name from a DELETE EDGE pair.
pub(super) fn extract_delete_edge_fields(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(u64, String), ParseError> {
    let mut edge_id: Option<u64> = None;
    let mut collection: Option<String> = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::value => edge_id = Some(parse_edge_id_value(inner)?),
            Rule::identifier => collection = Some(extract_identifier(&inner)),
            _ => {}
        }
    }

    Ok((
        require_field(edge_id, "DELETE EDGE", "an edge ID")?,
        require_field(collection, "DELETE EDGE", "a collection name")?,
    ))
}

/// Parses a value pair into a u64 edge ID.
fn parse_edge_id_value(pair: pest::iterators::Pair<Rule>) -> Result<u64, ParseError> {
    let v = Parser::parse_value(pair)?;
    extract_edge_id(&v)
}

/// Unwraps an optional field or returns a syntax error.
pub(super) fn require_field<T>(opt: Option<T>, context: &str, field: &str) -> Result<T, ParseError> {
    opt.ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires {field}")))
}

/// Parses edge field list: `identifier = value` pairs.
pub(super) fn parse_edge_fields(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Vec<(String, Value)>, ParseError> {
    super::helpers::extract_key_value_list(pair, Rule::edge_field, parse_single_edge_field)
}

/// Parses a single `edge_field`: `identifier = value`.
fn parse_single_edge_field(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(String, Value), ParseError> {
    let mut key = String::new();
    let mut value = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if key.is_empty() => {
                key = extract_identifier(&inner).to_ascii_lowercase();
            }
            Rule::value => value = Some(Parser::parse_value(inner)?),
            _ => {}
        }
    }

    let value = value.ok_or_else(|| ParseError::syntax(0, "", "Edge field requires a value"))?;
    Ok((key, value))
}

/// Parses the `edge_properties_clause`: `WITH PROPERTIES (options)`.
pub(super) fn parse_edge_properties(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Vec<(String, Value)>, ParseError> {
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::create_option_list {
            return super::helpers::extract_key_value_list(
                inner,
                Rule::create_option,
                parse_option_as_value,
            );
        }
    }

    Ok(Vec::new())
}

/// Parses a `create_option` into a `(String, Value)` pair.
fn parse_option_as_value(pair: pest::iterators::Pair<Rule>) -> Result<(String, Value), ParseError> {
    let mut key = String::new();
    let mut value = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::identifier if key.is_empty() => {
                key = extract_identifier(&inner).to_ascii_lowercase();
            }
            Rule::create_option_value => {
                value = Some(parse_create_option_value_as_value(inner)?);
            }
            _ => {}
        }
    }

    let value =
        value.ok_or_else(|| ParseError::syntax(0, "", "Property option requires a value"))?;
    Ok((key, value))
}

/// Converts a `create_option_value` into a `Value`.
fn parse_create_option_value_as_value(
    pair: pest::iterators::Pair<Rule>,
) -> Result<Value, ParseError> {
    let inner = pair
        .into_inner()
        .next()
        .ok_or_else(|| ParseError::syntax(0, "", "Expected option value"))?;

    match inner.as_rule() {
        Rule::identifier => Ok(Value::String(extract_identifier(&inner))),
        _ => crate::velesql::parser::helpers::parse_scalar_from_rule(&inner),
    }
}

/// Builds an `InsertEdgeStatement` from parsed fields and properties.
pub(super) fn build_insert_edge(
    collection: String,
    fields: &[(String, Value)],
    properties: Vec<(String, Value)>,
) -> Result<Query, ParseError> {
    let source = extract_required_u64(fields, "source", "INSERT EDGE")?;
    let target = extract_required_u64(fields, "target", "INSERT EDGE")?;
    let label = extract_required_string(fields, "label", "INSERT EDGE")?;
    let edge_id = extract_optional_u64(fields, "id");

    Ok(Query::new_dml(DmlStatement::InsertEdge(
        InsertEdgeStatement {
            collection,
            edge_id,
            source,
            target,
            label,
            properties,
        },
    )))
}

/// Extracts a required `u64` field from edge field pairs.
fn extract_required_u64(
    fields: &[(String, Value)],
    key: &str,
    context: &str,
) -> Result<u64, ParseError> {
    let value = fields
        .iter()
        .find(|(k, _)| k == key)
        .ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires '{key}' field")))?;

    extract_edge_id(&value.1)
}

/// Extracts a required `String` field from edge field pairs.
fn extract_required_string(
    fields: &[(String, Value)],
    key: &str,
    context: &str,
) -> Result<String, ParseError> {
    let value = fields
        .iter()
        .find(|(k, _)| k == key)
        .ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires '{key}' field")))?;

    match &value.1 {
        Value::String(s) => Ok(s.clone()),
        _ => Err(ParseError::syntax(
            0,
            "",
            format!("'{key}' must be a string"),
        )),
    }
}

/// Extracts an optional `u64` field from edge field pairs.
fn extract_optional_u64(fields: &[(String, Value)], key: &str) -> Option<u64> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| match v {
            Value::Integer(i) => u64::try_from(*i).ok(),
            Value::UnsignedInteger(u) => Some(*u),
            _ => None,
        })
}

/// Converts a `Value` reference to a `u64` edge ID.
fn extract_edge_id(value: &Value) -> Result<u64, ParseError> {
    match value {
        Value::Integer(i) => u64::try_from(*i)
            .map_err(|_| ParseError::syntax(0, "", "Edge ID must be a non-negative integer")),
        Value::UnsignedInteger(u) => Ok(*u),
        _ => Err(ParseError::syntax(0, "", "Edge ID must be an integer")),
    }
}

/// Builds an `InsertNodeStatement` from parsed fields.
///
/// Required fields: `id` (u64). Optional: `payload` (JSON string).
pub(super) fn build_insert_node(
    collection: String,
    fields: &[(String, Value)],
) -> Result<Query, ParseError> {
    let node_id = extract_required_u64(fields, "id", "INSERT NODE")?;
    let payload = build_node_payload(fields)?;

    Ok(Query::new_dml(DmlStatement::InsertNode(
        InsertNodeStatement {
            collection,
            node_id,
            payload,
        },
    )))
}

/// Builds the JSON payload for an `INSERT NODE` statement.
///
/// If a `payload` field is present as a JSON string, it is parsed and used
/// exclusively -- no other non-`id` fields are allowed alongside it.
fn build_node_payload(fields: &[(String, Value)]) -> Result<serde_json::Value, ParseError> {
    // Check for explicit payload field first.
    if let Some((_, payload_val)) = fields.iter().find(|(k, _)| k == "payload") {
        return match payload_val {
            Value::String(json_str) => {
                let extra_fields: Vec<&str> = fields
                    .iter()
                    .filter(|(k, _)| k != "id" && k != "payload")
                    .map(|(k, _)| k.as_str())
                    .collect();
                if !extra_fields.is_empty() {
                    return Err(ParseError::syntax(
                        0,
                        "",
                        format!(
                            "When 'payload' is specified as JSON string, other fields \
                             are ignored. Remove extra fields ({}) or omit the 'payload' field",
                            extra_fields.join(", ")
                        ),
                    ));
                }
                serde_json::from_str(json_str)
                    .map_err(|e| ParseError::syntax(0, "", format!("Invalid JSON in payload: {e}")))
            }
            _ => Err(ParseError::syntax(
                0,
                "",
                "The 'payload' field must be a JSON string \
                 (e.g., payload = '{\"key\": \"value\"}')",
            )),
        };
    }

    // Collect non-id fields into a JSON object.
    let mut map = serde_json::Map::new();
    for (key, value) in fields {
        if key == "id" {
            continue;
        }
        match value {
            Value::Parameter(_) | Value::Temporal(_) | Value::Subquery(_) => {
                return Err(ParseError::syntax(
                    0,
                    "",
                    "Node payload fields must be literal values",
                ));
            }
            _ => map.insert(key.clone(), value.to_json()),
        };
    }
    Ok(serde_json::Value::Object(map))
}
