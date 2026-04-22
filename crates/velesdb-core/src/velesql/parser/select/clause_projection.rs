//! SELECT list, column, and aggregate parsing.

use super::super::helpers::strip_identifier_quotes;
use super::super::{extract_identifier, Rule};
use super::validation;
use crate::velesql::ast::{
    AggregateArg, AggregateFunction, AggregateType, Column, SelectColumns, SimilarityScoreExpr,
    WindowFunction, WindowFunctionType, WindowOrderBy,
};
use crate::velesql::ast::OverClause;
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

/// Accumulator for parsed SELECT items before building `SelectColumns`.
struct SelectItemAccumulator {
    columns: Vec<Column>,
    aggregations: Vec<AggregateFunction>,
    similarity_scores: Vec<SimilarityScoreExpr>,
    qualified_wildcards: Vec<String>,
    window_functions: Vec<WindowFunction>,
}

impl SelectItemAccumulator {
    fn new() -> Self {
        Self {
            columns: Vec::new(),
            aggregations: Vec::new(),
            similarity_scores: Vec::new(),
            qualified_wildcards: Vec::new(),
            window_functions: Vec::new(),
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
            window_functions: self.window_functions,
        }
    }

    /// Counts how many distinct item types are present.
    fn count_nonempty_types(&self) -> usize {
        [
            !self.columns.is_empty(),
            !self.aggregations.is_empty(),
            !self.similarity_scores.is_empty(),
            !self.qualified_wildcards.is_empty(),
            !self.window_functions.is_empty(),
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
        // Multiple similarity scores, wildcards, or window functions without other types -> Mixed
        SelectColumns::Mixed {
            columns: self.columns,
            aggregations: self.aggregations,
            similarity_scores: self.similarity_scores,
            qualified_wildcards: self.qualified_wildcards,
            window_functions: self.window_functions,
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
                        Rule::window_item => {
                            acc.window_functions
                                .push(Self::parse_window_item(item)?);
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
                Rule::aggregate_arg => arg = Some(Self::parse_aggregate_arg(&inner_pair)),
                _ => {}
            }
        }
        Ok((
            agg_type.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate type"))?,
            arg.ok_or_else(|| ParseError::syntax(0, "", "Expected aggregate argument"))?,
        ))
    }

    pub(crate) fn parse_aggregate_arg(pair: &pest::iterators::Pair<Rule>) -> AggregateArg {
        let raw = pair.as_str().trim();
        if raw == "*" {
            return AggregateArg::Wildcard;
        }
        if raw.eq_ignore_ascii_case("score") {
            return AggregateArg::Score;
        }
        AggregateArg::Column(raw.to_string())
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
                .map(strip_identifier_quotes)
                .collect::<Vec<_>>()
                .join(".")
        } else {
            strip_identifier_quotes(raw)
        }
    }

    // ────────────────────────────────────────────────────────────
    // Window function parsing (Issue #386 Phase 1)
    // ────────────────────────────────────────────────────────────

    /// Parses a `window_item`: `window_function_call OVER (over_clause) [AS alias]`.
    pub(crate) fn parse_window_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<WindowFunction, ParseError> {
        let mut function_type = None;
        let mut over_clause = None;
        let mut alias = None;

        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::window_function_call => {
                    function_type = Some(Self::parse_window_function_call(inner_pair)?);
                }
                Rule::over_clause => {
                    over_clause = Some(Self::parse_over_clause(inner_pair)?);
                }
                Rule::identifier => {
                    alias = Some(extract_identifier(&inner_pair));
                }
                _ => {}
            }
        }

        Ok(WindowFunction {
            function_type: function_type
                .ok_or_else(|| ParseError::syntax(0, "", "Expected window function"))?,
            over_clause: over_clause
                .ok_or_else(|| ParseError::syntax(0, "", "Expected OVER clause"))?,
            alias,
        })
    }

    /// Parses `window_function_name ~ "(" ~ ")"`.
    fn parse_window_function_call(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<WindowFunctionType, ParseError> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::window_function_name {
                return match inner.as_str().to_uppercase().as_str() {
                    "ROW_NUMBER" => Ok(WindowFunctionType::RowNumber),
                    "RANK" => Ok(WindowFunctionType::Rank),
                    "DENSE_RANK" => Ok(WindowFunctionType::DenseRank),
                    other => Err(ParseError::syntax(
                        0,
                        other,
                        "Unknown window function",
                    )),
                };
            }
        }
        Err(ParseError::syntax(0, "", "Expected window function name"))
    }

    /// Parses `partition_by_clause? ~ window_order_by_clause?`.
    fn parse_over_clause(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<OverClause, ParseError> {
        let mut partition_by = Vec::new();
        let mut order_by = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::partition_by_clause => {
                    partition_by = Self::parse_partition_by_list(inner);
                }
                Rule::window_order_by_clause => {
                    order_by = Self::parse_window_order_by(inner)?;
                }
                _ => {}
            }
        }

        Ok(OverClause {
            partition_by,
            order_by,
        })
    }

    /// Extracts column names from `partition_by_clause`.
    fn parse_partition_by_list(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
        let mut cols = Vec::new();
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::partition_by_list {
                for col in inner.into_inner() {
                    if col.as_rule() == Rule::column_name {
                        cols.push(Self::parse_column_name(&col));
                    }
                }
            }
        }
        cols
    }

    /// Parses `window_order_by_clause` into a list of `WindowOrderBy`.
    fn parse_window_order_by(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Vec<WindowOrderBy>, ParseError> {
        let mut items = Vec::new();
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::window_order_by_item {
                items.push(Self::parse_window_order_by_item(inner)?);
            }
        }
        Ok(items)
    }

    /// Parses a single `window_order_by_item`.
    fn parse_window_order_by_item(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<WindowOrderBy, ParseError> {
        let mut column = None;
        let mut descending = false;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::window_order_by_expr => {
                    // The expression can be column_name or order_by_similarity_bare
                    for expr_inner in inner.into_inner() {
                        match expr_inner.as_rule() {
                            Rule::column_name => {
                                column = Some(Self::parse_column_name(&expr_inner));
                            }
                            Rule::order_by_similarity_bare => {
                                column = Some("similarity".to_string());
                            }
                            _ => {}
                        }
                    }
                }
                Rule::sort_direction => {
                    descending = inner.as_str().to_uppercase() == "DESC";
                }
                _ => {}
            }
        }

        Ok(WindowOrderBy {
            column: column.ok_or_else(|| {
                ParseError::syntax(0, "", "Expected column name in window ORDER BY")
            })?,
            descending,
        })
    }
}
