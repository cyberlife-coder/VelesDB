//! Value parsing (literals, parameters, WITH clause).

use super::helpers::{parse_scalar_from_rule, parse_u64_clause};
use super::Rule;
use crate::velesql::ast::{
    Condition, CorrelatedColumn, IntervalUnit, IntervalValue, Subquery, TemporalExpr, Value,
    WithClause, WithOption, WithValue,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

/// Converts a parsed scalar [`Value`] into a [`WithValue`].
///
/// WITH clause values use a parallel enum (`WithValue`) because they
/// support `Identifier` but not `Parameter`/`Temporal`/`Subquery`.
fn with_value_from_scalar(value: Value) -> Result<WithValue, ParseError> {
    match value {
        Value::String(s) => Ok(WithValue::String(s)),
        Value::Integer(v) => Ok(WithValue::Integer(v)),
        Value::UnsignedInteger(v) => {
            // WITH clause options use i64; values exceeding i64::MAX are rejected.
            let i = i64::try_from(v)
                .map_err(|_| ParseError::syntax(0, "", "WITH value exceeds i64::MAX"))?;
            Ok(WithValue::Integer(i))
        }
        Value::Float(v) => Ok(WithValue::Float(v)),
        Value::Boolean(b) => Ok(WithValue::Boolean(b)),
        _ => Err(ParseError::syntax(0, "", "Invalid WITH value type")),
    }
}

impl Parser {
    pub(crate) fn parse_value(pair: pest::iterators::Pair<Rule>) -> Result<Value, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected value"))?;

        // Temporal and subquery are value-level extensions not shared with
        // other scalar-parsing call sites, so handle them before delegation.
        match inner.as_rule() {
            Rule::temporal_expr => {
                let temporal = Self::parse_temporal_expr(inner)?;
                Ok(Value::Temporal(temporal))
            }
            Rule::subquery_expr => {
                let subquery = Self::parse_subquery_expr(inner)?;
                Ok(Value::Subquery(Box::new(subquery)))
            }
            _ => parse_scalar_from_rule(&inner),
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

        let interval_str =
            crate::velesql::parser::helpers::unescape_string_literal(string_pair.as_str());
        let interval_value = Self::parse_interval_string(&interval_str)?;
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
        let (left, is_subtract, right) = Self::extract_temporal_operands(pair)?;
        Ok(Self::build_temporal_expr(left, is_subtract, right))
    }

    /// Extracts the left operand, operator, and right operand from a temporal arithmetic node.
    fn extract_temporal_operands(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(TemporalExpr, bool, TemporalExpr), ParseError> {
        let parts: Vec<_> = pair.into_inner().collect();
        if parts.len() < 3 {
            return Err(ParseError::syntax(
                0,
                "",
                "Expected left operand, operator, right operand",
            ));
        }
        let left = Self::parse_temporal_operand(parts[0].clone())?;
        let is_subtract = parts[1].as_str() == "-";
        let right = Self::parse_temporal_operand(parts[2].clone())?;
        Ok((left, is_subtract, right))
    }

    /// Builds an Add or Subtract temporal expression from parsed operands.
    fn build_temporal_expr(
        left: TemporalExpr,
        is_subtract: bool,
        right: TemporalExpr,
    ) -> TemporalExpr {
        if is_subtract {
            TemporalExpr::Subtract(Box::new(left), Box::new(right))
        } else {
            TemporalExpr::Add(Box::new(left), Box::new(right))
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
        parse_u64_clause(pair, "LIMIT")
    }

    pub(crate) fn parse_offset_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<u64, ParseError> {
        parse_u64_clause(pair, "OFFSET")
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
            .to_ascii_lowercase();

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

        // Identifier is WITH-specific -- handle before scalar delegation.
        if inner.as_rule() == Rule::identifier {
            let s = super::extract_identifier(&inner);
            return Ok(WithValue::Identifier(s));
        }

        // Reuse shared scalar parsing, then convert Value -> WithValue.
        let value = parse_scalar_from_rule(&inner)?;
        with_value_from_scalar(value)
    }

    /// Parses a scalar subquery expression (EPIC-039).
    ///
    /// Handles subqueries like: `(SELECT AVG(price) FROM products WHERE category = 'tech')`
    pub(crate) fn parse_subquery_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Subquery, ParseError> {
        let select = Self::parse_subquery_select(pair)?;

        // EPIC-039 US-003: Detect correlated columns in WHERE clause
        let correlations =
            Self::detect_correlated_columns(select.where_clause.as_ref(), &select.from);

        Ok(Subquery {
            select,
            correlations,
        })
    }

    /// Parses the SELECT components of a subquery into a `SelectStatement`.
    fn parse_subquery_select(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<crate::velesql::ast::SelectStatement, ParseError> {
        use crate::velesql::ast::{GroupByClause, SelectColumns, SelectStatement};

        let mut columns = SelectColumns::All;
        let mut from = String::new();
        let mut where_clause = None;
        let mut group_by = None;
        let mut having = None;
        let mut limit = None;

        for sub_pair in pair.into_inner() {
            match sub_pair.as_rule() {
                Rule::select_list => columns = Self::parse_select_list(sub_pair)?,
                Rule::identifier => from = super::extract_identifier(&sub_pair),
                Rule::where_clause => where_clause = Some(Self::parse_where_clause(sub_pair)?),
                Rule::group_by_clause => {
                    let cols: Vec<String> = sub_pair
                        .into_inner()
                        .filter(|p| p.as_rule() == Rule::group_by_list)
                        .flat_map(pest::iterators::Pair::into_inner)
                        .map(|p| super::extract_identifier(&p))
                        .collect();
                    group_by = Some(GroupByClause { columns: cols });
                }
                Rule::having_clause => having = Some(Self::parse_having_clause(sub_pair)?),
                Rule::limit_clause => limit = Some(Self::parse_limit_clause(sub_pair)?),
                _ => {}
            }
        }

        Ok(SelectStatement {
            distinct: crate::velesql::DistinctMode::None,
            columns,
            from,
            from_alias: vec![],
            joins: Vec::new(),
            where_clause,
            order_by: None,
            limit,
            offset: None,
            with_clause: None,
            group_by,
            having,
            fusion_clause: None,
        })
    }

    /// Detects correlated columns in a subquery's WHERE clause (EPIC-039 US-003).
    ///
    /// A correlated column is a reference to an outer query's table/column,
    /// identified by a qualified name (e.g., `outer_table.column`).
    fn detect_correlated_columns(
        where_clause: Option<&Condition>,
        subquery_from: &str,
    ) -> Vec<CorrelatedColumn> {
        let mut correlations = Vec::new();

        if let Some(condition) = where_clause {
            Self::extract_correlations_from_condition(condition, subquery_from, &mut correlations);
        }

        correlations
    }

    /// Recursively extracts correlated column references from a condition.
    fn extract_correlations_from_condition(
        condition: &Condition,
        subquery_from: &str,
        correlations: &mut Vec<CorrelatedColumn>,
    ) {
        match condition {
            Condition::Comparison(comp) => {
                // Check left side for qualified column reference
                if let Some(corr) =
                    Self::extract_correlation_from_field(&comp.column, subquery_from)
                {
                    // Keep each outer table/column pair once in correlation metadata.
                    if !correlations.iter().any(|c| {
                        c.outer_table == corr.outer_table && c.outer_column == corr.outer_column
                    }) {
                        correlations.push(corr);
                    }
                }
                // Do not infer correlations from Value::String.
                // String literals (e.g., 'user@example.com', 'domain.name') are NOT
                // column references and should not be interpreted as table.column.
                // Column references on the right side would be represented as
                // Value::Parameter or a dedicated ColumnRef variant, not Value::String.
            }
            Condition::And(left, right) | Condition::Or(left, right) => {
                Self::extract_correlations_from_condition(left, subquery_from, correlations);
                Self::extract_correlations_from_condition(right, subquery_from, correlations);
            }
            Condition::Group(inner) | Condition::Not(inner) => {
                Self::extract_correlations_from_condition(inner, subquery_from, correlations);
            }
            _ => {}
        }
    }

    /// Extracts a correlated column from a qualified field reference.
    ///
    /// Returns Some(CorrelatedColumn) if the field is `outer_table.column`
    /// where `outer_table` is not the subquery's FROM table.
    fn extract_correlation_from_field(
        field: &str,
        subquery_from: &str,
    ) -> Option<CorrelatedColumn> {
        // Check for qualified name: table.column
        if let Some(dot_pos) = field.find('.') {
            let table = &field[..dot_pos];
            let column = &field[dot_pos + 1..];

            // If the table is NOT the subquery's own table, it's a correlation
            if !table.eq_ignore_ascii_case(subquery_from) && !table.is_empty() && !column.is_empty()
            {
                return Some(CorrelatedColumn {
                    outer_table: table.to_string(),
                    outer_column: column.to_string(),
                    inner_column: field.to_string(),
                });
            }
        }
        None
    }
}
