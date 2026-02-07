//! LIMIT, OFFSET, WITH options, and USING FUSION clause parsing.

use super::super::{extract_identifier, Rule};
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_using_fusion_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> crate::velesql::FusionClause {
        let mut strategy = crate::velesql::FusionStrategyType::Rrf;
        let mut k = None;
        let mut vector_weight = None;
        let mut graph_weight = None;
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::fusion_options {
                for opt_pair in inner_pair.into_inner() {
                    if opt_pair.as_rule() == Rule::fusion_option_list {
                        for option in opt_pair.into_inner() {
                            if option.as_rule() == Rule::fusion_option {
                                let mut key = String::new();
                                let mut value_str = String::new();
                                for part in option.into_inner() {
                                    match part.as_rule() {
                                        Rule::identifier => {
                                            key = extract_identifier(&part).to_lowercase()
                                        }
                                        Rule::fusion_value => {
                                            value_str = part.as_str().trim_matches('\'').to_string()
                                        }
                                        _ => {}
                                    }
                                }
                                match key.as_str() {
                                    "strategy" => {
                                        strategy = match value_str.to_lowercase().as_str() {
                                            "weighted" => {
                                                crate::velesql::FusionStrategyType::Weighted
                                            }
                                            "maximum" => {
                                                crate::velesql::FusionStrategyType::Maximum
                                            }
                                            _ => crate::velesql::FusionStrategyType::Rrf,
                                        }
                                    }
                                    "k" => k = value_str.parse().ok(),
                                    "vector_weight" => vector_weight = value_str.parse().ok(),
                                    "graph_weight" => graph_weight = value_str.parse().ok(),
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
        crate::velesql::FusionClause {
            strategy,
            k,
            vector_weight,
            graph_weight,
        }
    }
}
