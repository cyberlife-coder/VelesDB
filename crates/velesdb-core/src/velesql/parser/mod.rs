//! `VelesQL` parser implementation using pest.

mod conditions;
mod select;
mod values;

#[allow(dead_code)]
pub mod match_clause;
#[cfg(test)]
mod match_clause_tests;

use pest::Parser as PestParser;
use pest_derive::Parser;

use super::ast::Query;
use super::error::{ParseError, ParseErrorKind};

#[derive(Parser)]
#[grammar = "velesql/grammar.pest"]
pub(crate) struct VelesQLParser;

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
