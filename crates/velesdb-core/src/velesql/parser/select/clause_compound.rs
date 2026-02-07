//! Compound query and set operator parsing (UNION, INTERSECT, EXCEPT).

use super::super::Rule;
use crate::velesql::ast::{CompoundQuery, Query, SetOperator};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_compound_query(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Query, ParseError> {
        let mut select_stmts = Vec::new();
        let mut set_op = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::select_stmt => select_stmts.push(Self::parse_select_stmt(inner_pair)?),
                Rule::set_operator => set_op = Some(Self::parse_set_operator(inner_pair.as_str())),
                _ => {}
            }
        }
        let select = select_stmts
            .first()
            .cloned()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected SELECT statement"))?;
        let compound = if let (Some(op), Some(right)) = (set_op, select_stmts.get(1).cloned()) {
            Some(CompoundQuery {
                operator: op,
                right: Box::new(right),
            })
        } else {
            None
        };
        Ok(Query {
            select,
            compound,
            match_clause: None,
        })
    }

    fn parse_set_operator(text: &str) -> SetOperator {
        let upper = text.to_uppercase();
        if upper.contains("UNION") && upper.contains("ALL") {
            SetOperator::UnionAll
        } else if upper.contains("UNION") {
            SetOperator::Union
        } else if upper.contains("INTERSECT") {
            SetOperator::Intersect
        } else {
            SetOperator::Except
        }
    }
}
