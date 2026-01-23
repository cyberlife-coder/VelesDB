//! SELECT statement parsing.

use super::Rule;
use crate::velesql::ast::{
    Column, ColumnRef, JoinClause, JoinCondition, OrderByExpr, Query, SelectColumns, SelectOrderBy,
    SelectStatement, SimilarityOrderBy,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_query(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut inner = pair.into_inner();

        let select_pair = inner
            .find(|p| p.as_rule() == Rule::select_stmt)
            .ok_or_else(|| ParseError::syntax(0, "", "Expected SELECT statement"))?;

        let select = Self::parse_select_stmt(select_pair)?;

        Ok(Query { select })
    }

    pub(crate) fn parse_select_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectStatement, ParseError> {
        let mut columns = SelectColumns::All;
        let mut from = String::new();
        let mut joins = Vec::new();
        let mut where_clause = None;
        let mut order_by = None;
        let mut limit = None;
        let mut offset = None;
        let mut with_clause = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::select_list => {
                    columns = Self::parse_select_list(inner_pair)?;
                }
                Rule::identifier => {
                    from = inner_pair.as_str().to_string();
                }
                Rule::join_clause => {
                    joins.push(Self::parse_join_clause(inner_pair)?);
                }
                Rule::where_clause => {
                    where_clause = Some(Self::parse_where_clause(inner_pair)?);
                }
                Rule::order_by_clause => {
                    order_by = Some(Self::parse_order_by_clause(inner_pair)?);
                }
                Rule::limit_clause => {
                    limit = Some(Self::parse_limit_clause(inner_pair)?);
                }
                Rule::offset_clause => {
                    offset = Some(Self::parse_offset_clause(inner_pair)?);
                }
                Rule::with_clause => {
                    with_clause = Some(Self::parse_with_clause(inner_pair)?);
                }
                _ => {}
            }
        }

        Ok(SelectStatement {
            columns,
            from,
            joins,
            where_clause,
            order_by,
            limit,
            offset,
            with_clause,
        })
    }

    pub(crate) fn parse_order_by_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<SelectOrderBy>, ParseError> {
        let mut items = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::order_by_item {
                items.push(Self::parse_order_by_item(inner_pair)?);
            }
        }

        Ok(items)
    }

    pub(crate) fn parse_order_by_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectOrderBy, ParseError> {
        let mut expr = None;
        let mut descending = None;
        let mut is_similarity = false;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::order_by_expr => {
                    let (parsed_expr, sim) = Self::parse_order_by_expr(inner_pair)?;
                    expr = Some(parsed_expr);
                    is_similarity = sim;
                }
                Rule::sort_direction => {
                    let dir = inner_pair.as_str().to_uppercase();
                    descending = Some(dir == "DESC");
                }
                _ => {}
            }
        }

        let expr = expr.ok_or_else(|| ParseError::syntax(0, "", "Expected ORDER BY expression"))?;

        // Default: DESC for similarity (highest first), ASC for fields
        let descending = descending.unwrap_or(is_similarity);

        Ok(SelectOrderBy { expr, descending })
    }

    pub(crate) fn parse_order_by_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(OrderByExpr, bool), ParseError> {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::order_by_similarity => {
                    let sim = Self::parse_order_by_similarity(inner_pair)?;
                    return Ok((OrderByExpr::Similarity(sim), true));
                }
                Rule::identifier => {
                    return Ok((OrderByExpr::Field(inner_pair.as_str().to_string()), false));
                }
                _ => {}
            }
        }

        Err(ParseError::syntax(0, "", "Invalid ORDER BY expression"))
    }

    pub(crate) fn parse_order_by_similarity(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SimilarityOrderBy, ParseError> {
        let mut field = None;
        let mut vector = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::similarity_field => {
                    field = Some(inner_pair.as_str().to_string());
                }
                Rule::vector_value => {
                    vector = Some(Self::parse_vector_value(inner_pair)?);
                }
                _ => {}
            }
        }

        let field = field
            .ok_or_else(|| ParseError::syntax(0, "", "Expected field in ORDER BY similarity"))?;
        let vector = vector
            .ok_or_else(|| ParseError::syntax(0, "", "Expected vector in ORDER BY similarity"))?;

        Ok(SimilarityOrderBy { field, vector })
    }

    pub(crate) fn parse_select_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectColumns, ParseError> {
        let inner = pair.into_inner().next();

        match inner {
            Some(p) if p.as_rule() == Rule::column_list => {
                let columns = Self::parse_column_list(p)?;
                Ok(SelectColumns::Columns(columns))
            }
            _ => Ok(SelectColumns::All),
        }
    }

    pub(crate) fn parse_column_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<Column>, ParseError> {
        let mut columns = Vec::new();

        for col_pair in pair.into_inner() {
            if col_pair.as_rule() == Rule::column {
                columns.push(Self::parse_column(col_pair)?);
            }
        }

        Ok(columns)
    }

    pub(crate) fn parse_column(pair: pest::iterators::Pair<Rule>) -> Result<Column, ParseError> {
        let mut inner = pair.into_inner();
        let name_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;

        let name = Self::parse_column_name(&name_pair);
        let alias = inner.next().map(|p| p.as_str().to_string());

        Ok(Column { name, alias })
    }

    pub(crate) fn parse_column_name(pair: &pest::iterators::Pair<Rule>) -> String {
        // column_name is atomic (@), so we get the full string directly
        pair.as_str().to_string()
    }

    /// Parse JOIN clause (EPIC-031 US-004).
    pub(crate) fn parse_join_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<JoinClause, ParseError> {
        let mut table = String::new();
        let mut alias = None;
        let mut condition = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::identifier => {
                    table = inner_pair.as_str().to_string();
                }
                Rule::alias_clause => {
                    // alias_clause contains AS identifier
                    for alias_inner in inner_pair.into_inner() {
                        if alias_inner.as_rule() == Rule::identifier {
                            alias = Some(alias_inner.as_str().to_string());
                        }
                    }
                }
                Rule::join_condition => {
                    condition = Some(Self::parse_join_condition(inner_pair)?);
                }
                _ => {}
            }
        }

        let condition = condition
            .ok_or_else(|| ParseError::syntax(0, "", "JOIN clause missing ON condition"))?;

        Ok(JoinClause {
            table,
            alias,
            condition,
        })
    }

    /// Parse JOIN condition (table.column = var.property).
    pub(crate) fn parse_join_condition(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<JoinCondition, ParseError> {
        let mut refs: Vec<ColumnRef> = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::column_ref {
                refs.push(Self::parse_column_ref(&inner_pair)?);
            }
        }

        if refs.len() != 2 {
            return Err(ParseError::syntax(
                0,
                "",
                "JOIN condition requires exactly two column references",
            ));
        }

        let right = refs.pop().unwrap();
        let left = refs.pop().unwrap();

        Ok(JoinCondition { left, right })
    }

    /// Parse column reference (table.column).
    pub(crate) fn parse_column_ref(
        pair: &pest::iterators::Pair<Rule>,
    ) -> Result<ColumnRef, ParseError> {
        // column_ref is atomic (@), format: "table.column"
        let s = pair.as_str();
        let parts: Vec<&str> = s.split('.').collect();

        if parts.len() != 2 {
            return Err(ParseError::syntax(
                0,
                s,
                "Column reference must be in format 'table.column'",
            ));
        }

        Ok(ColumnRef {
            table: Some(parts[0].to_string()),
            column: parts[1].to_string(),
        })
    }
}
