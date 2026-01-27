//! `VelesQL` parser implementation using pest.

mod conditions;
mod select;
mod values;

#[allow(dead_code)]
pub mod match_clause;
#[cfg(test)]
mod match_clause_tests;
#[cfg(test)]
mod match_query_tests;
#[cfg(test)]
mod subquery_tests;
#[cfg(test)]
mod temporal_tests;

use pest::iterators::Pair;
use pest::Parser as PestParser;
use pest_derive::Parser;

use super::ast::Query;
use super::error::{ParseError, ParseErrorKind};

#[derive(Parser)]
#[grammar = "velesql/grammar.pest"]
pub(crate) struct VelesQLParser;

/// EPIC-044 US-005: Extract identifier string from any identifier form.
/// Handles: regular_identifier, backtick_identifier, doublequote_identifier
pub(crate) fn extract_identifier(pair: &Pair<'_, Rule>) -> String {
    match pair.as_rule() {
        Rule::identifier => {
            // identifier = { quoted_identifier | regular_identifier }
            if let Some(inner) = pair.clone().into_inner().next() {
                extract_identifier(&inner)
            } else {
                // Fallback for atomic match
                pair.as_str().to_string()
            }
        }
        Rule::quoted_identifier => {
            // quoted_identifier = { backtick_identifier | doublequote_identifier }
            if let Some(inner) = pair.clone().into_inner().next() {
                extract_identifier(&inner)
            } else {
                pair.as_str().to_string()
            }
        }
        Rule::backtick_identifier => {
            // Remove surrounding backticks: `name` -> name
            let s = pair.as_str();
            s[1..s.len() - 1].to_string()
        }
        Rule::doublequote_identifier => {
            // Remove surrounding quotes and unescape: "col""name" -> col"name
            let s = pair.as_str();
            let inner = &s[1..s.len() - 1];
            inner.replace("\"\"", "\"")
        }
        // Rule::regular_identifier and other rules: return as-is
        _ => pair.as_str().to_string(),
    }
}

/// `VelesQL` query parser.
pub struct Parser;

impl Parser {
    /// Parses a `VelesQL` query string into an AST.
    ///
    /// # Errors
    ///
    /// Returns a `ParseError` if the query is invalid.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use velesdb_core::velesql::Parser;
    ///
    /// let query = Parser::parse("SELECT * FROM documents LIMIT 10")?;
    /// ```
    pub fn parse(input: &str) -> Result<Query, ParseError> {
        let pairs = VelesQLParser::parse(Rule::query, input).map_err(|e| {
            let position = match e.location {
                pest::error::InputLocation::Pos(p) => p,
                pest::error::InputLocation::Span((s, _)) => s,
            };
            ParseError::new(
                ParseErrorKind::SyntaxError,
                position,
                input.chars().take(50).collect::<String>(),
                e.to_string(),
            )
        })?;

        let query_pair = pairs
            .into_iter()
            .next()
            .ok_or_else(|| ParseError::syntax(0, input, "Empty query"))?;

        Self::parse_query(query_pair)
    }
}
