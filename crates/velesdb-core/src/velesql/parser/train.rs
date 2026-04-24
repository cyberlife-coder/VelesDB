//! TRAIN QUANTIZER statement parsing.

use std::collections::HashMap;

use super::{extract_identifier, Rule};
use crate::velesql::ast::{Query, TrainStatement, WithValue};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_train_stmt(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut collection = None;
        let mut params = HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if collection.is_none() => {
                    collection = Some(extract_identifier(&inner));
                }
                Rule::with_clause => {
                    Self::collect_with_params(inner, &mut params)?;
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

    /// Parses a WITH clause and inserts its key-value options into `params`.
    fn collect_with_params(
        with_pair: pest::iterators::Pair<Rule>,
        params: &mut HashMap<String, WithValue>,
    ) -> Result<(), ParseError> {
        let with = Self::parse_with_clause(with_pair)?;
        for opt in with.options {
            params.insert(opt.key, opt.value);
        }
        Ok(())
    }
}
