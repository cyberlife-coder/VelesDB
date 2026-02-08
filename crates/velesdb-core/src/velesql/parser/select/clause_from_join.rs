//! FROM clause and JOIN parsing.

use super::super::{extract_identifier, Rule};
use crate::velesql::ast::{ColumnRef, JoinClause, JoinCondition};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_from_clause(pair: pest::iterators::Pair<Rule>) -> (String, Option<String>) {
        let mut table = String::new();
        let mut alias = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::identifier => {
                    if table.is_empty() {
                        table = extract_identifier(&inner_pair);
                    }
                }
                Rule::from_alias => {
                    for alias_inner in inner_pair.into_inner() {
                        if alias_inner.as_rule() == Rule::identifier {
                            alias = Some(extract_identifier(&alias_inner));
                        }
                    }
                }
                _ => {}
            }
        }
        (table, alias)
    }

    pub(crate) fn parse_join_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<JoinClause, ParseError> {
        let mut join_type = crate::velesql::JoinType::Inner;
        let mut table = String::new();
        let mut alias = None;
        let mut condition = None;
        let mut using_columns = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::join_type => join_type = Self::parse_join_type(inner_pair.as_str()),
                Rule::identifier => table = extract_identifier(&inner_pair),
                Rule::alias_clause => {
                    for alias_inner in inner_pair.into_inner() {
                        if alias_inner.as_rule() == Rule::identifier {
                            alias = Some(extract_identifier(&alias_inner));
                        }
                    }
                }
                Rule::join_spec => {
                    for spec_inner in inner_pair.into_inner() {
                        match spec_inner.as_rule() {
                            Rule::on_clause => {
                                for on_inner in spec_inner.into_inner() {
                                    if on_inner.as_rule() == Rule::join_condition {
                                        condition = Some(Self::parse_join_condition(on_inner)?);
                                    }
                                }
                            }
                            Rule::using_clause => {
                                using_columns = Some(
                                    spec_inner
                                        .into_inner()
                                        .filter(|p| p.as_rule() == Rule::identifier)
                                        .map(|p| extract_identifier(&p))
                                        .collect(),
                                );
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if condition.is_none() && using_columns.is_none() {
            return Err(ParseError::syntax(
                0,
                "",
                "JOIN clause requires ON or USING",
            ));
        }
        Ok(JoinClause {
            join_type,
            table,
            alias,
            condition,
            using_columns,
        })
    }

    fn parse_join_type(text: &str) -> crate::velesql::JoinType {
        let text = text.to_uppercase();
        if text.starts_with("LEFT") {
            crate::velesql::JoinType::Left
        } else if text.starts_with("RIGHT") {
            crate::velesql::JoinType::Right
        } else if text.starts_with("FULL") {
            crate::velesql::JoinType::Full
        } else {
            crate::velesql::JoinType::Inner
        }
    }

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
        let right = refs.pop().expect("right ref validated by len check");
        let left = refs.pop().expect("left ref validated by len check");
        Ok(JoinCondition { left, right })
    }

    pub(crate) fn parse_column_ref(
        pair: &pest::iterators::Pair<Rule>,
    ) -> Result<ColumnRef, ParseError> {
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
