//! SELECT statement parsing.

use super::Rule;
use crate::velesql::ast::{Column, Query, SelectColumns, SelectStatement};
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
        let mut where_clause = None;
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
                Rule::where_clause => {
                    where_clause = Some(Self::parse_where_clause(inner_pair)?);
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
            where_clause,
            limit,
            offset,
            with_clause,
        })
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
}
