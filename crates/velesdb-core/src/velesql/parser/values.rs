//! Value parsing (literals, parameters, WITH clause).

use super::Rule;
use crate::velesql::ast::{Value, WithClause, WithOption, WithValue};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_value(pair: pest::iterators::Pair<Rule>) -> Result<Value, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected value"))?;

        match inner.as_rule() {
            Rule::integer => {
                let v = inner
                    .as_str()
                    .parse::<i64>()
                    .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid integer"))?;
                Ok(Value::Integer(v))
            }
            Rule::float => {
                let v = inner
                    .as_str()
                    .parse::<f64>()
                    .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid float"))?;
                Ok(Value::Float(v))
            }
            Rule::string => {
                let s = inner.as_str().trim_matches('\'').to_string();
                Ok(Value::String(s))
            }
            Rule::boolean => {
                let b = inner.as_str().to_uppercase() == "TRUE";
                Ok(Value::Boolean(b))
            }
            Rule::null_value => Ok(Value::Null),
            Rule::parameter => {
                let name = inner.as_str().trim_start_matches('$').to_string();
                Ok(Value::Parameter(name))
            }
            _ => Err(ParseError::syntax(0, inner.as_str(), "Unknown value type")),
        }
    }

    pub(crate) fn parse_limit_clause(pair: pest::iterators::Pair<Rule>) -> Result<u64, ParseError> {
        let int_pair = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected integer for LIMIT"))?;

        int_pair
            .as_str()
            .parse::<u64>()
            .map_err(|_| ParseError::syntax(0, int_pair.as_str(), "Invalid LIMIT value"))
    }

    pub(crate) fn parse_offset_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<u64, ParseError> {
        let int_pair = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected integer for OFFSET"))?;

        int_pair
            .as_str()
            .parse::<u64>()
            .map_err(|_| ParseError::syntax(0, int_pair.as_str(), "Invalid OFFSET value"))
    }

    pub(crate) fn parse_with_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<WithClause, ParseError> {
        let mut options = Vec::new();

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::with_option_list {
                for opt_pair in inner_pair.into_inner() {
                    if opt_pair.as_rule() == Rule::with_option {
                        options.push(Self::parse_with_option(opt_pair)?);
                    }
                }
            }
        }

        Ok(WithClause { options })
    }

    pub(crate) fn parse_with_option(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<WithOption, ParseError> {
        let mut inner = pair.into_inner();

        let key = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected option key"))?
            .as_str()
            .to_string();

        let value_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected option value"))?;

        let value = Self::parse_with_value(value_pair)?;

        Ok(WithOption { key, value })
    }

    pub(crate) fn parse_with_value(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<WithValue, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected WITH value"))?;

        match inner.as_rule() {
            Rule::string => {
                let s = inner.as_str().trim_matches('\'').to_string();
                Ok(WithValue::String(s))
            }
            Rule::integer => {
                let v = inner
                    .as_str()
                    .parse::<i64>()
                    .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid integer"))?;
                Ok(WithValue::Integer(v))
            }
            Rule::float => {
                let v = inner
                    .as_str()
                    .parse::<f64>()
                    .map_err(|_| ParseError::syntax(0, inner.as_str(), "Invalid float"))?;
                Ok(WithValue::Float(v))
            }
            Rule::boolean => {
                let b = inner.as_str().to_uppercase() == "TRUE";
                Ok(WithValue::Boolean(b))
            }
            Rule::identifier => {
                let s = inner.as_str().to_string();
                Ok(WithValue::Identifier(s))
            }
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Invalid WITH value type",
            )),
        }
    }
}
