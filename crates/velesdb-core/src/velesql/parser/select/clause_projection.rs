//! SELECT list, column, and aggregate parsing.

use super::super::{extract_identifier, Rule};
use super::validation;
use crate::velesql::ast::{AggregateArg, AggregateFunction, AggregateType, Column, SelectColumns};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_select_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectColumns, ParseError> {
        let inner = pair.into_inner().next();
        match inner {
            Some(p) if p.as_rule() == Rule::select_item_list => {
                let (columns, aggs) = Self::parse_select_item_list(p)?;
                if aggs.is_empty() {
                    Ok(SelectColumns::Columns(columns))
                } else if columns.is_empty() {
                    Ok(SelectColumns::Aggregations(aggs))
                } else {
                    Ok(SelectColumns::Mixed {
                        columns,
                        aggregations: aggs,
                    })
                }
            }
            _ => Ok(SelectColumns::All),
        }
    }

    pub(crate) fn parse_select_item_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(Vec<Column>, Vec<AggregateFunction>), ParseError> {
        let mut columns = Vec::new();
        let mut aggs = Vec::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::select_item {
                for item in inner_pair.into_inner() {
                    match item.as_rule() {
                        Rule::aggregation_item => aggs.push(Self::parse_aggregation_item(item)?),
                        Rule::column => columns.push(Self::parse_column(item)?),
                        _ => {}
                    }
                }
            }
        }
        Ok((columns, aggs))
    }

    #[allow(dead_code)]
    pub(crate) fn parse_aggregation_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<AggregateFunction>, ParseError> {
        let mut aggs = Vec::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::aggregation_item {
                aggs.push(Self::parse_aggregation_item(inner_pair)?);
            }
        }
        Ok(aggs)
    }

    pub(crate) fn parse_aggregation_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<AggregateFunction, ParseError> {
        let mut function_type = None;
        let mut argument = None;
        let mut alias = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_function => {
                    let (ft, arg) = Self::parse_aggregate_function(inner_pair)?;
                    function_type = Some(ft);
                    argument = Some(arg);
                }
                Rule::identifier => alias = Some(extract_identifier(&inner_pair)),
                _ => {}
            }
        }
        Ok(AggregateFunction {
            function_type: function_type
                .ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate function"))?,
            argument: argument
                .ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?,
            alias,
        })
    }

    pub(crate) fn parse_aggregate_function(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(AggregateType, AggregateArg), ParseError> {
        let mut agg_type = None;
        let mut arg = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_type => {
                    agg_type = Some(validation::parse_aggregate_type(&inner_pair)?)
                }
                Rule::aggregate_arg => arg = Some(Self::parse_aggregate_arg(inner_pair)),
                _ => {}
            }
        }
        let agg_type =
            agg_type.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate type"))?;
        let arg = arg.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?;
        validation::validate_aggregate_wildcard(agg_type, &arg)?;
        Ok((agg_type, arg))
    }

    pub(crate) fn parse_aggregate_arg(pair: pest::iterators::Pair<Rule>) -> AggregateArg {
        let inner = pair.into_inner().next();
        match inner {
            Some(p) if p.as_rule() == Rule::column_name => {
                AggregateArg::Column(p.as_str().to_string())
            }
            _ => AggregateArg::Wildcard,
        }
    }

    #[allow(dead_code)]
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
        let alias = inner.next().map(|p| extract_identifier(&p));
        Ok(Column { name, alias })
    }

    pub(crate) fn parse_column_name(pair: &pest::iterators::Pair<Rule>) -> String {
        let raw = pair.as_str();
        Self::strip_quotes_from_column_name(raw)
    }

    fn strip_quotes_from_column_name(raw: &str) -> String {
        if raw.contains('.') {
            raw.split('.')
                .map(Self::strip_single_identifier_quotes)
                .collect::<Vec<_>>()
                .join(".")
        } else {
            Self::strip_single_identifier_quotes(raw)
        }
    }

    fn strip_single_identifier_quotes(s: &str) -> String {
        let s = s.trim();
        if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
            s[1..s.len() - 1].to_string()
        } else if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
            s[1..s.len() - 1].replace("\"\"\"", "\"")
        } else {
            s.to_string()
        }
    }
}
