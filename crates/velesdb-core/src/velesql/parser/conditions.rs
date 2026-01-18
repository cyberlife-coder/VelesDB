//! WHERE clause and condition parsing.

use super::Rule;
use crate::velesql::ast::{
    BetweenCondition, CompareOp, Comparison, Condition, FusionConfig, InCondition, IsNullCondition,
    LikeCondition, MatchCondition, VectorExpr, VectorFusedSearch, VectorSearch,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_where_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let or_expr = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        Self::parse_or_expr(or_expr)
    }

    pub(crate) fn parse_or_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner().peekable();

        let first = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        let mut result = Self::parse_and_expr(first)?;

        for and_expr in inner {
            let right = Self::parse_and_expr(and_expr)?;
            result = Condition::Or(Box::new(result), Box::new(right));
        }

        Ok(result)
    }

    pub(crate) fn parse_and_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner().peekable();

        let first = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected condition"))?;

        let mut result = Self::parse_primary_expr(first)?;

        for primary in inner {
            let right = Self::parse_primary_expr(primary)?;
            result = Condition::And(Box::new(result), Box::new(right));
        }

        Ok(result)
    }

    pub(crate) fn parse_primary_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected primary condition"))?;

        match inner.as_rule() {
            Rule::or_expr => {
                let cond = Self::parse_or_expr(inner)?;
                Ok(Condition::Group(Box::new(cond)))
            }
            Rule::vector_fused_search => Self::parse_vector_fused_search(inner),
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

    pub(crate) fn parse_vector_search(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut vector = None;

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::vector_value {
                vector = Some(Self::parse_vector_value(inner)?);
            }
        }

        let vector =
            vector.ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;

        Ok(Condition::VectorSearch(VectorSearch { vector }))
    }

    pub(crate) fn parse_vector_fused_search(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut vectors = Vec::new();
        let mut fusion = FusionConfig::default();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::vector_array => {
                    for vec_value in inner.into_inner() {
                        if vec_value.as_rule() == Rule::vector_value {
                            vectors.push(Self::parse_vector_value(vec_value)?);
                        }
                    }
                }
                Rule::fusion_clause => {
                    fusion = Self::parse_fusion_clause(inner);
                }
                _ => {}
            }
        }

        if vectors.is_empty() {
            return Err(ParseError::syntax(
                0,
                "",
                "Expected at least one vector in NEAR_FUSED",
            ));
        }

        Ok(Condition::VectorFusedSearch(VectorFusedSearch {
            vectors,
            fusion,
        }))
    }

    pub(crate) fn parse_fusion_clause(pair: pest::iterators::Pair<Rule>) -> FusionConfig {
        let mut strategy = "rrf".to_string();
        let mut params = std::collections::HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::fusion_strategy => {
                    strategy = inner.into_inner().next().map_or_else(
                        || "rrf".to_string(),
                        |s| s.as_str().trim_matches('\'').to_string(),
                    );
                }
                Rule::fusion_params => {
                    for param in inner.into_inner() {
                        if param.as_rule() == Rule::fusion_param_list {
                            for fp in param.into_inner() {
                                if fp.as_rule() == Rule::fusion_param {
                                    let mut fp_inner = fp.into_inner();
                                    if let (Some(key), Some(val)) =
                                        (fp_inner.next(), fp_inner.next())
                                    {
                                        let key_str = key.as_str().to_string();
                                        let val_f64 = val.as_str().parse::<f64>().unwrap_or(0.0);
                                        params.insert(key_str, val_f64);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        FusionConfig { strategy, params }
    }

    pub(crate) fn parse_vector_value(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<VectorExpr, ParseError> {
        let inner = pair
            .into_inner()
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected vector expression"))?;

        match inner.as_rule() {
            Rule::vector_literal => {
                let values: Result<Vec<f32>, _> = inner
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::float)
                    .map(|p| {
                        p.as_str()
                            .parse::<f32>()
                            .map_err(|_| ParseError::syntax(0, p.as_str(), "Invalid float value"))
                    })
                    .collect();
                Ok(VectorExpr::Literal(values?))
            }
            Rule::parameter => {
                let name = inner.as_str().trim_start_matches('$').to_string();
                Ok(VectorExpr::Parameter(name))
            }
            _ => Err(ParseError::syntax(
                0,
                inner.as_str(),
                "Expected vector literal or parameter",
            )),
        }
    }

    pub(crate) fn parse_match_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let query = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected match query"))?
            .as_str()
            .trim_matches('\'')
            .to_string();

        Ok(Condition::Match(MatchCondition { column, query }))
    }

    pub(crate) fn parse_in_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

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

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let low = Self::parse_value(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected low value"))?,
        )?;

        let high = Self::parse_value(
            inner
                .next()
                .ok_or_else(|| ParseError::syntax(0, "", "Expected high value"))?,
        )?;

        Ok(Condition::Between(BetweenCondition { column, low, high }))
    }

    pub(crate) fn parse_like_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Condition, ParseError> {
        let mut inner = pair.into_inner();

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

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
                Rule::identifier => {
                    column = inner.as_str().to_string();
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

        let column = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?
            .as_str()
            .to_string();

        let op_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected operator"))?;

        let operator = match op_pair.as_str() {
            "=" => CompareOp::Eq,
            "!=" | "<>" => CompareOp::NotEq,
            ">" => CompareOp::Gt,
            ">=" => CompareOp::Gte,
            "<" => CompareOp::Lt,
            "<=" => CompareOp::Lte,
            _ => return Err(ParseError::syntax(0, op_pair.as_str(), "Invalid operator")),
        };

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
}
