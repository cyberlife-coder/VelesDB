//! LIMIT, OFFSET, WITH options, and USING FUSION clause parsing.

use super::super::{extract_identifier, Rule};
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_using_fusion_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> crate::velesql::FusionClause {
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
                Self::parse_fusion_options(inner_pair, &mut clause);
            }
        }

        clause
    }

    /// Parses the fusion_options pair, iterating over each fusion_option.
    fn parse_fusion_options(
        pair: pest::iterators::Pair<Rule>,
        clause: &mut crate::velesql::FusionClause,
    ) {
        for opt_pair in pair.into_inner() {
            if opt_pair.as_rule() == Rule::fusion_option_list {
                for option in opt_pair.into_inner() {
                    if option.as_rule() == Rule::fusion_option {
                        Self::apply_fusion_option(option, clause);
                    }
                }
            }
        }
    }

    /// Parses a single fusion option key-value pair and applies it to the clause.
    fn apply_fusion_option(
        option: pest::iterators::Pair<Rule>,
        clause: &mut crate::velesql::FusionClause,
    ) {
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

        match key.as_str() {
            "strategy" => clause.strategy = Self::parse_fusion_strategy_type(&value_str),
            "k" => clause.k = value_str.parse().ok(),
            "vector_weight" => clause.vector_weight = value_str.parse().ok(),
            "graph_weight" => clause.graph_weight = value_str.parse().ok(),
            "dense_w" => clause.dense_weight = value_str.parse().ok(),
            "sparse_w" => clause.sparse_weight = value_str.parse().ok(),
            _ => {}
        }
    }

    /// Converts a strategy name string to a `FusionStrategyType`.
    fn parse_fusion_strategy_type(name: &str) -> crate::velesql::FusionStrategyType {
        match name.to_lowercase().as_str() {
            "weighted" => crate::velesql::FusionStrategyType::Weighted,
            "maximum" => crate::velesql::FusionStrategyType::Maximum,
            "rsf" => crate::velesql::FusionStrategyType::Rsf,
            "average" => crate::velesql::FusionStrategyType::Average,
            _ => crate::velesql::FusionStrategyType::Rrf,
        }
    }
}
