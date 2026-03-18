//! WHERE clause and condition parsing.

use super::{extract_identifier, Rule};
use crate::metrics::global_guardrails_metrics;
use crate::velesql::ast::{
    BetweenCondition, CompareOp, Comparison, Condition, InCondition, IsNullCondition,
    LikeCondition, MatchCondition, SimilarityCondition,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    const MAX_CONDITION_DEPTH: usize = 256;

    fn extract_column_name(pair: &pest::iterators::Pair<'_, Rule>) -> String {
        if pair.as_rule() != Rule::column_name && pair.as_rule() != Rule::where_column {
            return extract_identifier(pair);
        }

        let parts: Vec<String> = pair
            .clone()
            .into_inner()
            .filter(|p| p.as_rule() == Rule::identifier)
            .map(|p| extract_identifier(&p))
            .collect();

        if parts.is_empty() {
            pair.as_str().to_string()
        } else {
            parts.join(".")
        }
    }

    pub(crate) fn parse_where_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let or_expr = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        Self::parse_or_expr_with_depth(or_expr, 0)
    }

    #[allow(dead_code)]
    pub(crate) fn parse_or_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        Self::parse_or_expr_with_depth(pair, 0)
    }

    fn ensure_depth(depth: usize, input: &str) -> Result<(), ParseError> {
        if depth > Self::MAX_CONDITION_DEPTH {
            global_guardrails_metrics().record_parser_depth_limit_rejected();
            return Err(ParseError::syntax(0, input, "Condition nesting too deep"));
        }
        Ok(())
    }

    fn parse_or_expr_with_depth(
        pair: pest::iterators::Pair<Rule>,
        depth: usize,
    ) -> Result<Condition, ParseError> {
        Self::ensure_depth(depth, pair.as_str())?;
        let mut inner = pair.into_inner().peekable();

        let first = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        let mut result = Self::parse_and_expr_with_depth(first, depth + 1)?;

        for and_expr in inner {
            let right = Self::parse_and_expr_with_depth(and_expr, depth + 1)?;
            result = Condition::Or(Box::new(result), Box::new(right));
        }

        Ok(result)
    }

    #[allow(dead_code)]
    pub(crate) fn parse_and_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        Self::parse_and_expr_with_depth(pair, 0)
    }

    fn parse_and_expr_with_depth(
        pair: pest::iterators::Pair<Rule>,
        depth: usize,
    ) -> Result<Condition, ParseError> {
        Self::ensure_depth(depth, pair.as_str())?;
        let mut inner = pair.into_inner().peekable();

        let first = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        let mut result = Self::parse_primary_expr_with_depth(first, depth + 1)?;

        for primary in inner {
            let right = Self::parse_primary_expr_with_depth(primary, depth + 1)?;
            result = Condition::And(Box::new(result), Box::new(right));
        }

        Ok(result)
    }

    #[allow(dead_code)]
    pub(crate) fn parse_primary_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        Self::parse_primary_expr_with_depth(pair, 0)
    }

    fn parse_primary_expr_with_depth(
        pair: pest::iterators::Pair<Rule>,
        depth: usize,
    ) -> Result<Condition, ParseError> {
        Self::ensure_depth(depth, pair.as_str())?;
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected primary condition"))?;

        match inner.as_rule() {
            Rule::or_expr => {
                let cond = Self::parse_or_expr_with_depth(inner, depth + 1)?;
                Ok(Condition::Group(Box::new(cond)))
            }
            Rule::not_expr => {
                let nested = inner
                    .into_inner()
                    .next()
                    .ok_or_else(|| ParseError::syntax(0, "", "Expected expression after NOT"))?;
                let cond = Self::parse_primary_expr_with_depth(nested, depth + 1)?;
                Ok(Condition::Not(Box::new(cond)))
            }
            Rule::similarity_expr => Self::parse_similarity_expr(inner),
            Rule::graph_match_expr => Self::parse_graph_match_expr(inner),
            Rule::vector_fused_search => Self::parse_vector_fused_search(inner),
            Rule::sparse_vector_search => Self::parse_sparse_vector_search(inner),
            Rule::vector_search => Self::parse_vector_search(inner),
            Rule::match_expr => Self::parse_match_expr(inner),
            Rule::in_expr => Self::parse_in_expr(inner),
            Rule::between_expr => Self::parse_between_expr(inner),
            Rule::like_expr => Self::parse_like_expr(inner),
            Rule::is_null_expr => Self::parse_is_null_expr(inner),
            Rule::compare_expr => Self::parse_compare_expr(inner),
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Unknown condition type",
            )),
        }
    }

    /// Parses a similarity expression: `similarity(field, vector) op threshold`
    pub(crate) fn parse_similarity_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut field = None;
        let mut vector = None;
        let mut operator = None;
        let mut threshold = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::similarity_field => {
                    field = Some(inner.as_str().to_string());
                }
                Rule::vector_value => {
                    vector = Some(Self::parse_vector_value(inner)?);
                }
                Rule::compare_op => {
                    operator = Some(Self::parse_compare_op(inner.as_str())?);
                }
                Rule::numeric_threshold => {
                    // numeric_threshold = { float | integer }
                    let inner_value = inner
                        .into_inner()
                        .next()
                        .ok_or_else(|| ParseError::syntax(0, "", "Expected numeric threshold"))?;
                    threshold = Some(inner_value.as_str().parse::<f64>().map_err(|_| {
                        ParseError::syntax(0, inner_value.as_str(), "Invalid threshold")
                    })?);
                }
                _ => {}
            }
        }

        let field = field.ok_or_else(|| ParseError::syntax(0, "", "Expected field name"))?;
        let vector =
            vector.ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;
        let operator = operator.ok_or_else(|| ParseError::syntax(0, "", "Expected operator"))?;
        let threshold =
            threshold.ok_or_else(|| ParseError::syntax(0, "", "Expected threshold value"))?;

        Ok(Condition::Similarity(SimilarityCondition {
            field,
            vector,
            operator,
            threshold,
        }))
    }

    pub(crate) fn parse_match_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        let query = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected match query"))?
            .as_str()
            .trim_matches('\'')
            .to_string();

        Ok(Condition::Match(MatchCondition { column, query }))
    }

    pub(crate) fn parse_graph_match_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut graph_pattern = None;
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::graph_pattern {
                graph_pattern = Some(Self::parse_graph_pattern(inner)?);
            }
        }

        let pattern =
            graph_pattern.ok_or_else(|| ParseError::syntax(0, "", "Expected MATCH pattern"))?;
        Ok(Condition::GraphMatch(crate::velesql::GraphMatchPredicate {
            pattern,
        }))
    }

    pub(crate) fn parse_in_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        let value_list = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected value list"))?;

        let values: Result<Vec<_>, _> = value_list
            .into_inner()
            .filter(|p| p.as_rule() == Rule::value)
            .map(Self::parse_value)
            .collect();

        Ok(Condition::In(InCondition {
            column,
            values: values?,
        }))
    }

    pub(crate) fn parse_between_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        let low = Self::next_value(&mut inner, "Expected low value")?;
        let high = Self::next_value(&mut inner, "Expected high value")?;

        Ok(Condition::Between(BetweenCondition { column, low, high }))
    }

    /// Consumes the next pair from the iterator and parses it as a value.
    fn next_value(
        inner: &mut pest::iterators::Pairs<Rule>,
        error_msg: &str,
    ) -> Result<crate::velesql::ast::Value, ParseError> {
        let pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", error_msg))?;
        Self::parse_value(pair)
    }

    pub(crate) fn parse_like_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        // Parse LIKE or ILIKE operator
        let like_op = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected LIKE or ILIKE"))?
            .as_str()
            .to_uppercase();
        let case_insensitive = like_op == "ILIKE";

        let pattern = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected pattern"))?
            .as_str()
            .trim_matches('\'')
            .to_string();

        Ok(Condition::Like(LikeCondition {
            column,
            pattern,
            case_insensitive,
        }))
    }

    pub(crate) fn parse_is_null_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut column = String::new();
        let mut has_not = false;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier | Rule::column_name | Rule::where_column => {
                    column = Self::extract_column_name(&inner);
                }
                Rule::not_kw => {
                    has_not = true;
                }
                _ => {}
            }
        }

        if column.is_empty() {
            return Err(ParseError::syntax(0, "", "Expected column name in IS NULL"));
        }

        Ok(Condition::IsNull(IsNullCondition {
            column,
            is_null: !has_not,
        }))
    }

    pub(crate) fn parse_compare_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let column = Self::extract_column_name(&column_pair);

        let op_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected operator"))?;
        let operator = Self::parse_compare_op(op_pair.as_str())?;

        let value = Self::parse_value(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected value"))?,
        )?;

        Ok(Condition::Comparison(Comparison {
            column,
            operator,
            value,
        }))
    }

    /// Parses a comparison operator string into a `CompareOp`.
    fn parse_compare_op(op: &str) -> Result<CompareOp, ParseError> {
        match op {
            "=" => Ok(CompareOp::Eq),
            "!=" | "<>" => Ok(CompareOp::NotEq),
            ">" => Ok(CompareOp::Gt),
            ">=" => Ok(CompareOp::Gte),
            "<" => Ok(CompareOp::Lt),
            "<=" => Ok(CompareOp::Lte),
            _ => Err(ParseError::syntax(0, op, "Invalid operator")),
        }
    }
}
