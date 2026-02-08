//! GROUP BY, HAVING, and ORDER BY clause parsing.

use super::super::{extract_identifier, Rule};
use super::validation;
use crate::velesql::ast::{
    AggregateFunction, GroupByClause, HavingClause, HavingCondition, OrderByExpr, SelectOrderBy,
    SimilarityOrderBy,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_group_by_clause(pair: pest::iterators::Pair<Rule>) -> GroupByClause {
        let mut columns = Vec::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::group_by_list {
                for col_pair in inner_pair.into_inner() {
                    if col_pair.as_rule() == Rule::group_by_column {
                        let parts: Vec<String> = col_pair
                            .into_inner()
                            .filter(|p| p.as_rule() == Rule::identifier)
                            .map(|p| extract_identifier(&p))
                            .collect();
                        columns.push(parts.join("."));
                    }
                }
            }
        }
        GroupByClause { columns }
    }

    pub(crate) fn parse_having_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<HavingClause, ParseError> {
        let mut conditions = Vec::new();
        let mut operators = Vec::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::having_condition {
                for term_pair in inner_pair.into_inner() {
                    match term_pair.as_rule() {
                        Rule::having_term => conditions.push(Self::parse_having_term(term_pair)?),
                        Rule::having_logical_op => {
                            let text = term_pair.as_str().to_uppercase();
                            if text == "AND" {
                                operators.push(crate::velesql::LogicalOp::And);
                            } else if text == "OR" {
                                operators.push(crate::velesql::LogicalOp::Or);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(HavingClause {
            conditions,
            operators,
        })
    }

    fn parse_having_term(pair: pest::iterators::Pair<Rule>) -> Result<HavingCondition, ParseError> {
        let mut aggregate = None;
        let mut operator = None;
        let mut value = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_function => {
                    aggregate = Some(Self::parse_aggregate_function_only(inner_pair)?);
                }
                Rule::compare_op => operator = Some(validation::parse_compare_op(&inner_pair)?),
                Rule::value => value = Some(Self::parse_value(inner_pair)?),
                _ => {}
            }
        }
        Ok(HavingCondition {
            aggregate: aggregate
                .ok_or_else(|| ParseError::syntax(0, "", "HAVING requires aggregate function"))?,
            operator: operator
                .ok_or_else(|| ParseError::syntax(0, "", "HAVING requires comparison operator"))?,
            value: value.ok_or_else(|| ParseError::syntax(0, "", "HAVING requires value"))?,
        })
    }

    pub(crate) fn parse_aggregate_function_only(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<AggregateFunction, ParseError> {
        let (function_type, argument) = Self::parse_aggregate_function(pair)?;
        Ok(AggregateFunction {
            function_type,
            argument,
            alias: None,
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
                    descending = Some(inner_pair.as_str().to_uppercase() == "DESC");
                }
                _ => {}
            }
        }
        let expr = expr.ok_or_else(|| ParseError::syntax(0, "", "Expected ORDER BY expression"))?;
        Ok(SelectOrderBy {
            expr,
            descending: descending.unwrap_or(is_similarity),
        })
    }

    pub(crate) fn parse_order_by_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(OrderByExpr, bool), ParseError> {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::order_by_similarity => {
                    return Ok((
                        OrderByExpr::Similarity(Self::parse_order_by_similarity(inner_pair)?),
                        true,
                    ));
                }
                Rule::aggregate_function => {
                    return Ok((
                        OrderByExpr::Aggregate(Self::parse_aggregate_function_only(inner_pair)?),
                        false,
                    ));
                }
                Rule::identifier => {
                    return Ok((OrderByExpr::Field(extract_identifier(&inner_pair)), false))
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
                Rule::similarity_field => field = Some(inner_pair.as_str().to_string()),
                Rule::vector_value => vector = Some(Self::parse_vector_value(inner_pair)?),
                _ => {}
            }
        }
        Ok(SimilarityOrderBy {
            field: field.ok_or_else(|| {
                ParseError::syntax(0, "", "Expected field in ORDER BY similarity")
            })?,
            vector: vector.ok_or_else(|| {
                ParseError::syntax(0, "", "Expected vector in ORDER BY similarity")
            })?,
        })
    }
}
