//! SELECT statement parsing - facade module.

mod clause_compound;
mod clause_from_join;
mod clause_group_order;
mod clause_limit_with;
mod clause_projection;
pub(crate) mod validation;

use super::Rule;
use crate::velesql::ast::{Query, SelectColumns, SelectStatement};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_query(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let inner = pair.into_inner();
        for p in inner {
            match p.as_rule() {
                Rule::match_query => return Self::parse_match_query(p),
                Rule::compound_query => return Self::parse_compound_query(p),
                Rule::insert_stmt => return Self::parse_insert_stmt(p),
                Rule::update_stmt => return Self::parse_update_stmt(p),
                _ => {}
            }
        }
        Err(ParseError::syntax(
            0,
            "",
            "Expected MATCH, SELECT, INSERT, or UPDATE query",
        ))
    }

    pub(crate) fn parse_select_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectStatement, ParseError> {
        let mut distinct = crate::velesql::DistinctMode::None;
        let mut columns = SelectColumns::All;
        let mut from = String::new();
        let mut from_alias = None;
        let mut joins = Vec::new();
        let mut where_clause = None;
        let mut order_by = None;
        let mut limit = None;
        let mut offset = None;
        let mut with_clause = None;
        let mut group_by = None;
        let mut having = None;
        let mut fusion_clause = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::distinct_modifier => distinct = crate::velesql::DistinctMode::All,
                Rule::select_list => columns = Self::parse_select_list(inner_pair)?,
                Rule::from_clause => {
                    let (table, alias) = Self::parse_from_clause(inner_pair);
                    from = table;
                    from_alias = alias;
                }
                Rule::join_clause => joins.push(Self::parse_join_clause(inner_pair)?),
                Rule::where_clause => where_clause = Some(Self::parse_where_clause(inner_pair)?),
                Rule::group_by_clause => group_by = Some(Self::parse_group_by_clause(inner_pair)),
                Rule::having_clause => having = Some(Self::parse_having_clause(inner_pair)?),
                Rule::order_by_clause => order_by = Some(Self::parse_order_by_clause(inner_pair)?),
                Rule::limit_clause => limit = Some(Self::parse_limit_clause(inner_pair)?),
                Rule::offset_clause => offset = Some(Self::parse_offset_clause(inner_pair)?),
                Rule::with_clause => with_clause = Some(Self::parse_with_clause(inner_pair)?),
                Rule::using_fusion_clause => {
                    fusion_clause = Some(Self::parse_using_fusion_clause(inner_pair));
                }
                _ => {}
            }
        }
        Ok(SelectStatement {
            distinct,
            columns,
            from,
            from_alias,
            joins,
            where_clause,
            order_by,
            limit,
            offset,
            with_clause,
            group_by,
            having,
            fusion_clause,
        })
    }
}
