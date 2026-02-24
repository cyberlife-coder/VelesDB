//! FROM clause and JOIN parsing.

use super::super::{extract_identifier, Rule};
use crate::velesql::ast::{ColumnRef, JoinClause, JoinCondition};
use crate::velesql::error::{ParseError, ParseErrorKind};
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
        let pair_start = pair.as_span().start();
        let pair_text = pair.as_str().to_string();
        let mut refs = pair
            .into_inner()
            .filter(|inner_pair| inner_pair.as_rule() == Rule::column_ref)
            .map(|inner_pair| Self::parse_column_ref(&inner_pair));

        let left = refs.next().transpose()?.ok_or_else(|| {
            ParseError::new(
                ParseErrorKind::SyntaxError,
                pair_start,
                pair_text.clone(),
                "Expected left-side column reference in JOIN condition.".to_string(),
            )
        })?;

        let right = refs.next().transpose()?.ok_or_else(|| {
            ParseError::new(
                ParseErrorKind::SyntaxError,
                pair_start,
                pair_text.clone(),
                "Expected right-side column reference in JOIN condition.".to_string(),
            )
        })?;

        if refs.next().is_some() {
            return Err(ParseError::new(
                ParseErrorKind::SyntaxError,
                pair_start,
                pair_text,
                "JOIN condition must contain exactly two column references.".to_string(),
            ));
        }

        Ok(JoinCondition { left, right })
    }

    pub(crate) fn parse_column_ref(
        pair: &pest::iterators::Pair<Rule>,
    ) -> Result<ColumnRef, ParseError> {
        let s = pair.as_str();
        let (table, column) = Self::split_column_ref(s).ok_or_else(|| {
            ParseError::new(
                ParseErrorKind::SyntaxError,
                pair.as_span().start(),
                s,
                "Column reference must be in format 'table.column'.".to_string(),
            )
        })?;

        Ok(ColumnRef {
            table: Some(table),
            column,
        })
    }

    fn split_column_ref(input: &str) -> Option<(String, String)> {
        let mut separator_index = None;
        let mut chars = input.char_indices().peekable();
        let mut in_backtick = false;
        let mut in_double_quotes = false;

        while let Some((index, ch)) = chars.next() {
            if in_backtick {
                if ch == '`' {
                    in_backtick = false;
                }
                continue;
            }

            if in_double_quotes {
                if ch == '"' {
                    if matches!(chars.peek(), Some((_, '"'))) {
                        chars.next();
                    } else {
                        in_double_quotes = false;
                    }
                }
                continue;
            }

            match ch {
                '`' => in_backtick = true,
                '"' => in_double_quotes = true,
                '.' => {
                    if separator_index.replace(index).is_some() {
                        return None;
                    }
                }
                _ => {}
            }
        }

        if in_backtick || in_double_quotes {
            return None;
        }

        let dot_index = separator_index?;
        let (left, right_with_dot) = input.split_at(dot_index);
        let right = right_with_dot.strip_prefix('.')?;

        if left.is_empty() || right.is_empty() {
            return None;
        }

        Some((
            Self::normalize_identifier(left),
            Self::normalize_identifier(right),
        ))
    }

    fn normalize_identifier(identifier: &str) -> String {
        if identifier.starts_with('`') && identifier.ends_with('`') && identifier.len() >= 2 {
            return identifier[1..identifier.len() - 1].to_string();
        }

        if identifier.starts_with('"') && identifier.ends_with('"') && identifier.len() >= 2 {
            return identifier[1..identifier.len() - 1].replace("\"\"", "\"");
        }

        identifier.to_string()
    }
}
