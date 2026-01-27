//! Value parsing (literals, parameters, WITH clause).

use super::Rule;
use crate::velesql::ast::{
    IntervalUnit, IntervalValue, Subquery, TemporalExpr, Value, WithClause, WithOption, WithValue,
};
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
            Rule::temporal_expr => {
                let temporal = Self::parse_temporal_expr(inner)?;
                Ok(Value::Temporal(temporal))
            }
            Rule::subquery_expr => {
                let subquery = Self::parse_subquery_expr(inner)?;
                Ok(Value::Subquery(Box::new(subquery)))
            }
            _ => Err(ParseError::syntax(0, inner.as_str(), "Unknown value type")),
        }
    }

    /// Parses a temporal expression (NOW(), INTERVAL, arithmetic).
    pub(crate) fn parse_temporal_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<TemporalExpr, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected temporal expression"))?;

        match inner.as_rule() {
            Rule::now_function => Ok(TemporalExpr::Now),
            Rule::interval_expr => Self::parse_interval_expr(inner),
            Rule::temporal_arithmetic => Self::parse_temporal_arithmetic(inner),
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Unknown temporal expression",
            )),
        }
    }

    /// Parses an INTERVAL expression like `INTERVAL '7 days'`.
    fn parse_interval_expr(pair: pest::iterators::Pair<Rule>) -> Result<TemporalExpr, ParseError> {
        let string_pair = pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::string)
            .ok_or_else(|| ParseError::syntax(0, "", "Expected interval string"))?;

        let interval_str = string_pair.as_str().trim_matches('\'');
        let interval_value = Self::parse_interval_string(interval_str)?;
        Ok(TemporalExpr::Interval(interval_value))
    }

    /// Parses interval string like "7 days" or "1 hour".
    fn parse_interval_string(s: &str) -> Result<IntervalValue, ParseError> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(ParseError::syntax(
                0,
                s,
                "INTERVAL format: '<number> <unit>' (e.g., '7 days')",
            ));
        }

        let magnitude = parts[0]
            .parse::<i64>()
            .map_err(|_| ParseError::syntax(0, parts[0], "Invalid interval magnitude"))?;

        let unit = match parts[1].to_lowercase().as_str() {
            "s" | "sec" | "second" | "seconds" => IntervalUnit::Seconds,
            "m" | "min" | "minute" | "minutes" => IntervalUnit::Minutes,
            "h" | "hour" | "hours" => IntervalUnit::Hours,
            "d" | "day" | "days" => IntervalUnit::Days,
            "w" | "week" | "weeks" => IntervalUnit::Weeks,
            "month" | "months" => IntervalUnit::Months,
            _ => {
                return Err(ParseError::syntax(
                    0,
                    parts[1],
                    "Unknown interval unit (use: seconds, minutes, hours, days, weeks, months)",
                ))
            }
        };

        Ok(IntervalValue { magnitude, unit })
    }

    /// Parses temporal arithmetic like `NOW() - INTERVAL '7 days'`.
    fn parse_temporal_arithmetic(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<TemporalExpr, ParseError> {
        let mut inner = pair.into_inner();

        let left_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected left operand"))?;
        let left = Self::parse_temporal_operand(left_pair)?;

        let op_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected temporal operator"))?;
        let is_subtract = op_pair.as_str() == "-";

        let right_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected right operand"))?;
        let right = Self::parse_temporal_operand(right_pair)?;

        if is_subtract {
            Ok(TemporalExpr::Subtract(Box::new(left), Box::new(right)))
        } else {
            Ok(TemporalExpr::Add(Box::new(left), Box::new(right)))
        }
    }

    /// Parses a single temporal operand (NOW or INTERVAL).
    fn parse_temporal_operand(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<TemporalExpr, ParseError> {
        match pair.as_rule() {
            Rule::now_function => Ok(TemporalExpr::Now),
            Rule::interval_expr => Self::parse_interval_expr(pair),
            _ => Err(ParseError::syntax(
                0,
                pair.as_str(),
                "Expected NOW() or INTERVAL",
            )),
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
                let s = super::extract_identifier(&inner);
                Ok(WithValue::Identifier(s))
            }
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Invalid WITH value type",
            )),
        }
    }

    /// Parses a scalar subquery expression (EPIC-039).
    ///
    /// Handles subqueries like: `(SELECT AVG(price) FROM products WHERE category = 'tech')`
    pub(crate) fn parse_subquery_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Subquery, ParseError> {
        use crate::velesql::ast::{GroupByClause, SelectColumns, SelectStatement};

        let inner = pair.into_inner();
        let mut columns = SelectColumns::All;
        let mut from = String::new();
        let mut where_clause = None;
        let mut group_by = None;
        let mut having = None;
        let mut limit = None;

        // Parse each component of the subquery
        for sub_pair in inner {
            match sub_pair.as_rule() {
                Rule::select_list => {
                    columns = Self::parse_select_list(sub_pair)?;
                }
                Rule::identifier => {
                    from = super::extract_identifier(&sub_pair);
                }
                Rule::where_clause => {
                    where_clause = Some(Self::parse_where_clause(sub_pair)?);
                }
                Rule::group_by_clause => {
                    let cols: Vec<String> = sub_pair
                        .into_inner()
                        .filter(|p| p.as_rule() == Rule::group_by_list)
                        .flat_map(pest::iterators::Pair::into_inner)
                        .map(|p| super::extract_identifier(&p))
                        .collect();
                    group_by = Some(GroupByClause { columns: cols });
                }
                Rule::having_clause => {
                    having = Some(Self::parse_having_clause(sub_pair)?);
                }
                Rule::limit_clause => {
                    limit = Some(Self::parse_limit_clause(sub_pair)?);
                }
                _ => {}
            }
        }

        let select = SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns,
            from,
            joins: Vec::new(),
            where_clause,
            order_by: None,
            limit,
            offset: None,
            with_clause: None,
            group_by,
            having,
            fusion_clause: None,
        };

        // TODO: Detect correlated columns by analyzing column references
        let correlations = Vec::new();

        Ok(Subquery {
            select,
            correlations,
        })
    }
}
