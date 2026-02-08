//! Shared cross-cutting validation rules for SELECT parsing.

use super::super::Rule;
use crate::velesql::ast::{AggregateArg, AggregateType, CompareOp};
use crate::velesql::error::ParseError;

/// Parse aggregate type keyword (COUNT, SUM, AVG, MIN, MAX).
pub(crate) fn parse_aggregate_type(
    pair: &pest::iterators::Pair<Rule>,
) -> Result<AggregateType, ParseError> {
    let type_str = pair.as_str().to_uppercase();
    match type_str.as_str() {
        "COUNT" => Ok(AggregateType::Count),
        "SUM" => Ok(AggregateType::Sum),
        "AVG" => Ok(AggregateType::Avg),
        "MIN" => Ok(AggregateType::Min),
        "MAX" => Ok(AggregateType::Max),
        other => Err(ParseError::syntax(0, other, "Unknown aggregate function")),
    }
}

/// Validate that wildcard (*) is only used with COUNT.
pub(crate) fn validate_aggregate_wildcard(
    agg_type: AggregateType,
    arg: &AggregateArg,
) -> Result<(), ParseError> {
    if matches!(arg, AggregateArg::Wildcard) && !matches!(agg_type, AggregateType::Count) {
        return Err(ParseError::syntax(
            0,
            format!("{agg_type:?}(*)"),
            format!(
                "{agg_type:?}(*) is invalid - only COUNT(*) accepts *. \
                 Use {agg_type:?}(column_name) instead"
            ),
        ));
    }
    Ok(())
}

/// Parse comparison operator token into `CompareOp`.
pub(crate) fn parse_compare_op(
    pair: &pest::iterators::Pair<Rule>,
) -> Result<CompareOp, ParseError> {
    match pair.as_str() {
        "=" => Ok(CompareOp::Eq),
        "!=" | "<>" => Ok(CompareOp::NotEq),
        ">" => Ok(CompareOp::Gt),
        ">=" => Ok(CompareOp::Gte),
        "<" => Ok(CompareOp::Lt),
        "<=" => Ok(CompareOp::Lte),
        other => Err(ParseError::syntax(0, other, "Unknown comparison operator")),
    }
}
