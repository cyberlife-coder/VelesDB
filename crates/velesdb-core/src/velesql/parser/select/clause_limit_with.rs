//! LIMIT, OFFSET, WITH options, and USING FUSION clause parsing.

use super::super::{extract_identifier, Rule};
use crate::velesql::error::{ParseError, ParseErrorKind};
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_using_fusion_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<crate::velesql::FusionClause, ParseError> {
        let mut clause = crate::velesql::FusionClause {
            strategy: crate::velesql::FusionStrategyType::Rrf,
            k: None,
            vector_weight: None,
            graph_weight: None,
            dense_weight: None,
            sparse_weight: None,
        };

        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::fusion_options {
                Self::parse_fusion_options(inner_pair, &mut clause)?;
            }
        }

        Ok(clause)
    }

    /// Parses the fusion_options pair, iterating over each fusion_option.
    fn parse_fusion_options(
        pair: pest::iterators::Pair<Rule>,
        clause: &mut crate::velesql::FusionClause,
    ) -> Result<(), ParseError> {
        for opt_pair in pair.into_inner() {
            if opt_pair.as_rule() == Rule::fusion_option_list {
                for option in opt_pair.into_inner() {
                    if option.as_rule() == Rule::fusion_option {
                        Self::apply_fusion_option(option, clause)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Extracts the `(key, value)` strings of a single fusion option pair.
    fn extract_fusion_kv(option: pest::iterators::Pair<Rule>) -> (String, String) {
        let mut key = String::new();
        let mut value_str = String::new();
        for part in option.into_inner() {
            match part.as_rule() {
                Rule::identifier => key = extract_identifier(&part).to_lowercase(),
                Rule::fusion_value => {
                    // fusion_value = { string | float | integer }
                    // Only unescape if the inner child is a string literal.
                    if let Some(child) = part.into_inner().next() {
                        value_str = if child.as_rule() == Rule::string {
                            crate::velesql::parser::helpers::unescape_string_literal(child.as_str())
                        } else {
                            child.as_str().to_string()
                        };
                    }
                }
                _ => {}
            }
        }
        (key, value_str)
    }

    /// Parses a single fusion option key-value pair and applies it to the clause.
    ///
    /// Unknown keys are rejected (a misspelled weight must not silently no-op);
    /// `dense_weight`/`sparse_weight` are accepted as long-name aliases of
    /// `dense_w`/`sparse_w`.
    fn apply_fusion_option(
        option: pest::iterators::Pair<Rule>,
        clause: &mut crate::velesql::FusionClause,
    ) -> Result<(), ParseError> {
        let (key, value_str) = Self::extract_fusion_kv(option);

        match key.as_str() {
            "strategy" => clause.strategy = Self::parse_fusion_strategy_type(&value_str)?,
            "k" => clause.k = value_str.parse().ok(),
            "vector_weight" => clause.vector_weight = value_str.parse().ok(),
            "graph_weight" => clause.graph_weight = value_str.parse().ok(),
            "dense_w" | "dense_weight" => clause.dense_weight = value_str.parse().ok(),
            "sparse_w" | "sparse_weight" => clause.sparse_weight = value_str.parse().ok(),
            other => {
                return Err(ParseError::new(
                    ParseErrorKind::SyntaxError,
                    0,
                    other.to_string(),
                    format!(
                        "Unknown USING FUSION option '{other}'. Valid keys: strategy, k, \
                         vector_weight, graph_weight, dense_weight, sparse_weight"
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Converts a strategy name string to a `FusionStrategyType`.
    ///
    /// `relative_score` is accepted as an alias of `rsf`. Unknown strategy
    /// names are rejected instead of silently falling back to RRF.
    fn parse_fusion_strategy_type(
        name: &str,
    ) -> Result<crate::velesql::FusionStrategyType, ParseError> {
        match name.to_lowercase().as_str() {
            "rrf" => Ok(crate::velesql::FusionStrategyType::Rrf),
            "weighted" => Ok(crate::velesql::FusionStrategyType::Weighted),
            "maximum" => Ok(crate::velesql::FusionStrategyType::Maximum),
            "rsf" | "relative_score" => Ok(crate::velesql::FusionStrategyType::Rsf),
            "average" => Ok(crate::velesql::FusionStrategyType::Average),
            other => Err(ParseError::new(
                ParseErrorKind::SyntaxError,
                0,
                other.to_string(),
                format!(
                    "Unknown USING FUSION strategy '{other}'. Valid strategies: rrf, weighted, \
                     maximum, rsf (relative_score), average"
                ),
            )),
        }
    }
}
