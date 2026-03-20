//! SELECT list, column, and aggregate parsing.

use super::super::{extract_identifier, Rule};
use super::validation;
use crate::velesql::ast::{
    AggregateArg, AggregateFunction, AggregateType, Column, SelectColumns, SimilarityScoreExpr,
};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

/// Accumulator for parsed SELECT items before building `SelectColumns`.
struct SelectItemAccumulator {
    columns: Vec<Column>,
    aggregations: Vec<AggregateFunction>,
    similarity_scores: Vec<SimilarityScoreExpr>,
    qualified_wildcards: Vec<String>,
}

impl SelectItemAccumulator {
    fn new() -> Self {
        Self {
            columns: Vec::new(),
            aggregations: Vec::new(),
            similarity_scores: Vec::new(),
            qualified_wildcards: Vec::new(),
        }
    }

    fn into_select_columns(self) -> SelectColumns {
        let type_count = self.count_nonempty_types();

        // Single-type shorthand: exactly one kind of item present
        if type_count == 1 {
            return self.into_single_type();
        }

        // Mixed: 2+ item types
        SelectColumns::Mixed {
            columns: self.columns,
            aggregations: self.aggregations,
            similarity_scores: self.similarity_scores,
            qualified_wildcards: self.qualified_wildcards,
        }
    }

    /// Counts how many distinct item types are present.
    fn count_nonempty_types(&self) -> usize {
        [
            !self.columns.is_empty(),
            !self.aggregations.is_empty(),
            !self.similarity_scores.is_empty(),
            !self.qualified_wildcards.is_empty(),
        ]
        .iter()
        .filter(|&&b| b)
        .count()
    }

    /// Converts when exactly one item type is present.
    /// Falls back to `Mixed` for multi-element similarity/wildcard.
    fn into_single_type(self) -> SelectColumns {
        if !self.columns.is_empty() {
            return SelectColumns::Columns(self.columns);
        }
        if !self.aggregations.is_empty() {
            return SelectColumns::Aggregations(self.aggregations);
        }
        if self.similarity_scores.len() == 1 {
            return SelectColumns::SimilarityScore(
                self.similarity_scores
                    .into_iter()
                    .next()
                    .expect("checked len==1"),
            );
        }
        if self.qualified_wildcards.len() == 1 {
            return SelectColumns::QualifiedWildcard(
                self.qualified_wildcards
                    .into_iter()
                    .next()
                    .expect("checked len==1"),
            );
        }
        // Multiple similarity scores or wildcards without other types → Mixed
        SelectColumns::Mixed {
            columns: self.columns,
            aggregations: self.aggregations,
            similarity_scores: self.similarity_scores,
            qualified_wildcards: self.qualified_wildcards,
        }
    }
}

impl Parser {
    pub(crate) fn parse_select_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectColumns, ParseError> {
        let inner = pair.into_inner().next();
        match inner {
            Some(p) if p.as_rule() == Rule::select_item_list => Self::parse_select_item_list(p),
            _ => Ok(SelectColumns::All),
        }
    }

    pub(crate) fn parse_select_item_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectColumns, ParseError> {
        let mut acc = SelectItemAccumulator::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::select_item {
                for item in inner_pair.into_inner() {
                    match item.as_rule() {
                        Rule::similarity_select => {
                            acc.similarity_scores
                                .push(Self::parse_similarity_select(item));
                        }
                        Rule::aggregation_item => {
                            acc.aggregations.push(Self::parse_aggregation_item(item)?);
                        }
                        Rule::qualified_wildcard => {
                            acc.qualified_wildcards
                                .push(Self::parse_qualified_wildcard(item));
                        }
                        Rule::column => acc.columns.push(Self::parse_column(item)?),
                        _ => {}
                    }
                }
            }
        }
        Ok(acc.into_select_columns())
    }

    /// Parses `similarity() [AS alias]`.
    pub(crate) fn parse_similarity_select(
        pair: pest::iterators::Pair<Rule>,
    ) -> SimilarityScoreExpr {
        let mut alias = None;
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::identifier {
                alias = Some(extract_identifier(&inner_pair));
            }
        }
        SimilarityScoreExpr { alias }
    }

    /// Parses `alias.*` qualified wildcard.
    pub(crate) fn parse_qualified_wildcard(pair: pest::iterators::Pair<Rule>) -> String {
        let mut alias = String::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::identifier {
                alias = extract_identifier(&inner_pair);
            }
        }
        alias
    }

    #[allow(dead_code)]
    pub(crate) fn parse_aggregation_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<AggregateFunction>, ParseError> {
        let mut aggs = Vec::new();
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::aggregation_item {
                aggs.push(Self::parse_aggregation_item(inner_pair)?);
            }
        }
        Ok(aggs)
    }

    pub(crate) fn parse_aggregation_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<AggregateFunction, ParseError> {
        let mut function_type = None;
        let mut argument = None;
        let mut alias = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_function => {
                    let (ft, arg) = Self::parse_aggregate_function(inner_pair)?;
                    function_type = Some(ft);
                    argument = Some(arg);
                }
                Rule::identifier => alias = Some(extract_identifier(&inner_pair)),
                _ => {}
            }
        }
        Ok(AggregateFunction {
            function_type: function_type
                .ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate function"))?,
            argument: argument
                .ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?,
            alias,
        })
    }

    pub(crate) fn parse_aggregate_function(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(AggregateType, AggregateArg), ParseError> {
        let (agg_type, arg) = Self::extract_aggregate_parts(pair)?;
        validation::validate_aggregate_wildcard(agg_type, &arg)?;
        Ok((agg_type, arg))
    }

    /// Extracts the aggregate type and argument from an `aggregate_function` node.
    fn extract_aggregate_parts(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(AggregateType, AggregateArg), ParseError> {
        let mut agg_type = None;
        let mut arg = None;
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::aggregate_type => {
                    agg_type = Some(validation::parse_aggregate_type(&inner_pair)?);
                }
                Rule::aggregate_arg => arg = Some(Self::parse_aggregate_arg(inner_pair)),
                _ => {}
            }
        }
        Ok((
            agg_type.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate type"))?,
            arg.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?,
        ))
    }

    pub(crate) fn parse_aggregate_arg(pair: pest::iterators::Pair<Rule>) -> AggregateArg {
        let inner = pair.into_inner().next();
        match inner {
            Some(p) if p.as_rule() == Rule::column_name => {
                AggregateArg::Column(p.as_str().to_string())
            }
            _ => AggregateArg::Wildcard,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn parse_column_list(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<Column>, ParseError> {
        let mut columns = Vec::new();
        for col_pair in pair.into_inner() {
            if col_pair.as_rule() == Rule::column {
                columns.push(Self::parse_column(col_pair)?);
            }
        }
        Ok(columns)
    }

    pub(crate) fn parse_column(pair: pest::iterators::Pair<Rule>) -> Result<Column, ParseError> {
        let mut inner = pair.into_inner();
        let name_pair = inner
            .next()
            .ok_or_else(|| ParseError::syntax(0, "", "Expected column name"))?;
        let name = Self::parse_column_name(&name_pair);
        let alias = inner.next().map(|p| extract_identifier(&p));
        Ok(Column { name, alias })
    }

    pub(crate) fn parse_column_name(pair: &pest::iterators::Pair<Rule>) -> String {
        let raw = pair.as_str();
        Self::strip_quotes_from_column_name(raw)
    }

    fn strip_quotes_from_column_name(raw: &str) -> String {
        if raw.contains('.') {
            raw.split('.')
                .map(Self::strip_single_identifier_quotes)
                .collect::<Vec<_>>()
                .join(".")
        } else {
            Self::strip_single_identifier_quotes(raw)
        }
    }

    fn strip_single_identifier_quotes(s: &str) -> String {
        let s = s.trim();
        if s.starts_with('`') && s.ends_with('`') && s.len() >= 2 {
            s[1..s.len() - 1].to_string()
        } else if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
            s[1..s.len() - 1].replace("\"\"\"", "\"")
        } else {
            s.to_string()
        }
    }
}
