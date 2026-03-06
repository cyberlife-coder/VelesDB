//! TRAIN QUANTIZER statement parsing.

use std::collections::HashMap;

use super::{extract_identifier, Rule};
use crate::velesql::ast::{Query, TrainStatement};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_train_stmt(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut collection = None;
        let mut params = HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if collection.is_none() {
                        collection = Some(extract_identifier(&inner));
                    }
                }
                Rule::with_clause => {
                    let with = Self::parse_with_clause(inner)?;
                    for opt in with.options {
                        params.insert(opt.key, opt.value);
                    }
                }
                _ => {}
            }
        }

        let collection = collection
            .ok_or_else(|| ParseError::syntax(0, "", "TRAIN QUANTIZER requires collection name"))?;

        if params.is_empty() {
            return Err(ParseError::syntax(
                0,
                "",
                "TRAIN QUANTIZER requires at least one WITH parameter",
            ));
        }

        Ok(Query::new_train(TrainStatement { collection, params }))
    }
}
