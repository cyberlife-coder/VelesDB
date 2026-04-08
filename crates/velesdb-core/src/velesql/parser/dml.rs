//! DML statement parsing (INSERT, UPDATE, INSERT EDGE, DELETE, DELETE EDGE,
//! SELECT EDGES, INSERT NODE).

use super::{extract_identifier, Rule};
use crate::velesql::ast::{
    Condition, DeleteEdgeStatement, DeleteStatement, DmlStatement, InsertStatement, Query,
    SelectEdgesStatement, UpdateAssignment, UpdateStatement, Value,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

use super::dml_helpers::{
    build_insert_edge, build_insert_node, extract_delete_edge_fields, extract_delete_fields,
    parse_edge_fields, parse_edge_properties, validate_insert_rows,
};

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
    #[allow(clippy::type_complexity)] // SAFETY: one-off tuple for internal parser helper.
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

        Self::build_update_query(table, assignments, where_clause)
    }

    /// Validates extracted UPDATE components and builds the query.
    fn build_update_query(
        table: Option<String>,
        assignments: Vec<UpdateAssignment>,
        where_clause: Option<Condition>,
    ) -> Result<Query, ParseError> {
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
