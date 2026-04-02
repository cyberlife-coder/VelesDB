//! DML statement parsing (INSERT, UPDATE, INSERT EDGE, DELETE, DELETE EDGE,
//! SELECT EDGES, INSERT NODE).

use super::{extract_identifier, Rule};
use crate::velesql::ast::{
    Condition, DeleteEdgeStatement, DeleteStatement, DmlStatement, InsertEdgeStatement,
    InsertNodeStatement, InsertStatement, Query, SelectEdgesStatement, UpdateAssignment,
    UpdateStatement, Value,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    /// Parses an `INSERT INTO ... VALUES ...` statement (supports multi-row).
    pub(crate) fn parse_insert_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let (table, columns, rows) = Self::parse_insert_or_upsert_body(pair, "INSERT")?;
        Ok(Query::new_dml(DmlStatement::Insert(InsertStatement {
            table,
            columns,
            rows,
        })))
    }

    /// Parses an `UPSERT INTO ... VALUES ...` statement (supports multi-row).
    pub(crate) fn parse_upsert_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let (table, columns, rows) = Self::parse_insert_or_upsert_body(pair, "UPSERT")?;
        Ok(Query::new_dml(DmlStatement::Upsert(InsertStatement {
            table,
            columns,
            rows,
        })))
    }

    /// Shared body parser for INSERT and UPSERT statements.
    ///
    /// Extracts collection name, column list, and one or more value rows from a
    /// `insert_stmt` or `upsert_stmt` grammar pair.
    #[allow(clippy::type_complexity)] // Reason: one-off tuple for internal parser helper.
    fn parse_insert_or_upsert_body(
        pair: pest::iterators::Pair<Rule>,
        context: &str,
    ) -> Result<(String, Vec<String>, Vec<Vec<Value>>), ParseError> {
        let mut table = None;
        let mut columns = Vec::new();
        let mut rows = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if table.is_none() {
                        table = Some(extract_identifier(&inner));
                    } else {
                        columns.push(extract_identifier(&inner));
                    }
                }
                Rule::values_row => rows.push(Self::parse_values_row(inner)?),
                _ => {}
            }
        }

        let table = table.ok_or_else(|| {
            ParseError::syntax(0, "", format!("{context} requires target collection"))
        })?;
        validate_insert_rows(&columns, &rows, context)?;
        Ok((table, columns, rows))
    }

    /// Parses a single `values_row`: `(v1, v2, ...)`.
    fn parse_values_row(pair: pest::iterators::Pair<Rule>) -> Result<Vec<Value>, ParseError> {
        pair.into_inner()
            .filter(|p| p.as_rule() == Rule::value)
            .map(Self::parse_value)
            .collect()
    }

    pub(crate) fn parse_update_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut table = None;
        let mut assignments = Vec::new();
        let mut where_clause = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if table.is_none() => {
                    table = Some(extract_identifier(&inner));
                }
                Rule::assignment => {
                    assignments.push(Self::parse_assignment(inner)?);
                }
                Rule::where_clause => where_clause = Some(Self::parse_where_clause(inner)?),
                _ => {}
            }
        }

        let table =
            table.ok_or_else(|| ParseError::syntax(0, "", "UPDATE requires target collection"))?;
        if assignments.is_empty() {
            return Err(ParseError::syntax(
                0,
                "",
                "UPDATE requires at least one assignment",
            ));
        }

        Ok(Query::new_dml(DmlStatement::Update(UpdateStatement {
            table,
            assignments,
            where_clause,
        })))
    }

    /// Parses a single `column = value` assignment from an UPDATE statement.
    fn parse_assignment(pair: pest::iterators::Pair<Rule>) -> Result<UpdateAssignment, ParseError> {
        let mut inner = pair.into_inner();
        let column = inner
            .next()
            .map(|p| extract_identifier(&p))
            .ok_or_else(|| ParseError::syntax(0, "", "UPDATE assignment missing column"))?;
        let value_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "UPDATE assignment missing value"))?;
        let value = Self::parse_value(value_pair)?;
        Ok(UpdateAssignment { column, value })
    }

    /// Parses an `INSERT EDGE` statement.
    ///
    /// Grammar:
    /// ```text
    /// INSERT EDGE INTO collection (fields) [WITH PROPERTIES (options)]
    /// ```
    pub(crate) fn parse_insert_edge_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut collection: Option<String> = None;
        let mut fields: Vec<(String, Value)> = Vec::new();
        let mut properties: Vec<(String, Value)> = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if collection.is_none() => {
                    collection = Some(extract_identifier(&inner));
                }
                Rule::edge_field_list => fields = parse_edge_fields(inner)?,
                Rule::edge_properties_clause => {
                    properties = parse_edge_properties(inner)?;
                }
                _ => {}
            }
        }

        let collection = collection
            .ok_or_else(|| ParseError::syntax(0, "", "INSERT EDGE requires a collection name"))?;

        build_insert_edge(collection, &fields, properties)
    }

    /// Parses a `DELETE FROM` statement.
    ///
    /// Grammar:
    /// ```text
    /// DELETE FROM collection WHERE condition
    /// ```
    pub(crate) fn parse_delete_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let (table, where_clause) = extract_delete_fields(pair)?;

        Ok(Query::new_dml(DmlStatement::Delete(DeleteStatement {
            table,
            where_clause,
        })))
    }

    /// Parses a `DELETE EDGE` statement.
    ///
    /// Grammar:
    /// ```text
    /// DELETE EDGE edge_id FROM collection
    /// ```
    pub(crate) fn parse_delete_edge_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let (edge_id, collection) = extract_delete_edge_fields(pair)?;

        Ok(Query::new_dml(DmlStatement::DeleteEdge(
            DeleteEdgeStatement {
                collection,
                edge_id,
            },
        )))
    }
    /// Parses a `SELECT EDGES` statement.
    ///
    /// Grammar:
    /// ```text
    /// SELECT EDGES FROM collection [WHERE ...] [LIMIT n]
    /// ```
    pub(crate) fn parse_select_edges_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut collection: Option<String> = None;
        let mut where_clause = None;
        let mut limit = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if collection.is_none() => {
                    collection = Some(extract_identifier(&inner));
                }
                Rule::where_clause => where_clause = Some(Self::parse_where_clause(inner)?),
                Rule::limit_clause => limit = Some(Self::parse_limit_clause(inner)?),
                _ => {}
            }
        }

        let collection = collection
            .ok_or_else(|| ParseError::syntax(0, "", "SELECT EDGES requires a collection name"))?;

        Ok(Query::new_dml(DmlStatement::SelectEdges(
            SelectEdgesStatement {
                collection,
                where_clause,
                limit,
            },
        )))
    }

    /// Parses an `INSERT NODE` statement.
    ///
    /// Grammar:
    /// ```text
    /// INSERT NODE INTO collection (id = N, payload = '{"key": "value"}')
    /// ```
    pub(crate) fn parse_insert_node_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut collection: Option<String> = None;
        let mut fields: Vec<(String, Value)> = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if collection.is_none() => {
                    collection = Some(extract_identifier(&inner));
                }
                Rule::edge_field_list => fields = parse_edge_fields(inner)?,
                _ => {}
            }
        }

        let collection = collection
            .ok_or_else(|| ParseError::syntax(0, "", "INSERT NODE requires a collection name"))?;

        build_insert_node(collection, &fields)
    }
}

// ---------------------------------------------------------------------------
// Private helpers for DML extensions
// ---------------------------------------------------------------------------

/// Validates multi-row INSERT/UPSERT: non-empty columns, at least one row,
/// and every row length matches the column count.
fn validate_insert_rows(
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
///
/// Grammar guarantees at most one identifier and one where_clause,
/// so guards are unnecessary.
fn extract_delete_fields(
    pair: pest::iterators::Pair<Rule>,
) -> Result<(String, Condition), ParseError> {
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
///
/// Grammar guarantees at most one value and one identifier,
/// so guards are unnecessary.
fn extract_delete_edge_fields(
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
fn require_field<T>(opt: Option<T>, context: &str, field: &str) -> Result<T, ParseError> {
    opt.ok_or_else(|| ParseError::syntax(0, "", format!("{context} requires {field}")))
}

/// Parses edge field list: `identifier = value` pairs.
fn parse_edge_fields(
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
fn parse_edge_properties(
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
fn build_insert_edge(
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
/// Any other key-value pairs are collected into the payload object.
fn build_insert_node(collection: String, fields: &[(String, Value)]) -> Result<Query, ParseError> {
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
/// If `payload` is present but not a string, a parse error is returned.
/// Otherwise, all non-`id` fields are collected into a JSON object.
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
