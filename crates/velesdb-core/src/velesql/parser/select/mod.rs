//! SELECT statement parsing - facade module.

mod clause_compound;
mod clause_from_join;
mod clause_group_order;
mod clause_limit_with;
mod clause_projection;
pub(crate) mod validation;

use super::Rule;
use crate::velesql::ast::{LetBinding, Query, SelectColumns, SelectStatement};
use crate::velesql::error::ParseError;
use crate::velesql::Parser;

impl Parser {
    pub(crate) fn parse_query(pair: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        let mut let_bindings = Vec::new();
        for p in pair.into_inner() {
            if p.as_rule() == Rule::let_clause {
                let_bindings.push(Self::parse_let_clause(p)?);
            } else {
                let mut q = Self::dispatch_statement(p)?;
                q.let_bindings = let_bindings;
                return Ok(q);
            }
        }
        Err(ParseError::syntax(
            0,
            "",
            "Expected SHOW, DESCRIBE, EXPLAIN, ANALYZE, TRUNCATE, ALTER, FLUSH, MATCH, SELECT, INSERT, UPSERT, UPDATE, DELETE, CREATE, DROP, or TRAIN query",
        ))
    }

    /// Routes a top-level statement pair to the appropriate parser method.
    ///
    /// Each arm delegates to a dedicated `parse_*` method that owns the full
    /// parsing logic for that statement type. This keeps `parse_query` at CC 3.
    fn dispatch_statement(p: pest::iterators::Pair<Rule>) -> Result<Query, ParseError> {
        match p.as_rule() {
            Rule::show_collections_stmt => Self::parse_show_collections_stmt(p),
            Rule::describe_stmt => Self::parse_describe_stmt(p),
            Rule::explain_stmt => Self::parse_explain_stmt(p),
            Rule::analyze_stmt => Self::parse_analyze_stmt(p),
            Rule::truncate_stmt => Self::parse_truncate_stmt(p),
            Rule::alter_collection_stmt => Self::parse_alter_collection_stmt(p),
            Rule::flush_stmt => Self::parse_flush_stmt(p),
            Rule::match_query => Self::parse_match_query(p),
            Rule::compound_query => Self::parse_compound_query(p),
            Rule::train_stmt => Self::parse_train_stmt(p),
            Rule::create_index_stmt => Self::parse_create_index_stmt(p),
            Rule::create_collection_stmt => Self::parse_create_collection_stmt(p),
            Rule::drop_index_stmt => Self::parse_drop_index_stmt(p),
            Rule::drop_collection_stmt => Self::parse_drop_collection_stmt(p),
            Rule::select_edges_stmt => Self::parse_select_edges_stmt(p),
            Rule::insert_node_stmt => Self::parse_insert_node_stmt(p),
            Rule::insert_edge_stmt => Self::parse_insert_edge_stmt(p),
            Rule::delete_edge_stmt => Self::parse_delete_edge_stmt(p),
            Rule::delete_stmt => Self::parse_delete_stmt(p),
            Rule::insert_stmt => Self::parse_insert_stmt(p),
            Rule::upsert_stmt => Self::parse_upsert_stmt(p),
            Rule::update_stmt => Self::parse_update_stmt(p),
            _ => Err(ParseError::syntax(0, "", "Unknown statement type")),
        }
    }

    /// Parses a single `LET name = expr` clause (VelesQL v1.10 Phase 3).
    fn parse_let_clause(pair: pest::iterators::Pair<Rule>) -> Result<LetBinding, ParseError> {
        let (name, expr) = Self::extract_let_name_and_expr(pair)?;
        Ok(LetBinding { name, expr })
    }

    /// Extracts the name and arithmetic expression from a LET clause pair.
    ///
    /// Grammar guarantees exactly one `identifier` and one `order_by_arithmetic`.
    fn extract_let_name_and_expr(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<(String, crate::velesql::ArithmeticExpr), ParseError> {
        let mut inner = pair.into_inner();
        let name = inner
            .find(|p| p.as_rule() == Rule::identifier)
            .map(|p| super::extract_identifier(&p))
            .ok_or_else(|| ParseError::syntax(0, "", "LET clause requires a binding name"))?;
        let expr_pair = inner
            .find(|p| p.as_rule() == Rule::order_by_arithmetic)
            .ok_or_else(|| ParseError::syntax(0, "", "LET clause requires an expression"))?;
        let (parsed, _) = Self::parse_order_by_arithmetic(expr_pair)?;
        Ok((name, Self::order_by_to_arithmetic(parsed)))
    }

    /// Converts an `OrderByExpr` back to `ArithmeticExpr` for LET storage.
    ///
    /// `parse_order_by_arithmetic` collapses single-atom arithmetic expressions
    /// to legacy `OrderByExpr::Field` / `OrderByExpr::SimilarityBare` variants.
    /// LET bindings always store `ArithmeticExpr`, so we reverse that collapse.
    fn order_by_to_arithmetic(expr: crate::velesql::OrderByExpr) -> crate::velesql::ArithmeticExpr {
        match expr {
            crate::velesql::OrderByExpr::Field(name) => {
                crate::velesql::ArithmeticExpr::Variable(name)
            }
            crate::velesql::OrderByExpr::SimilarityBare => {
                crate::velesql::ArithmeticExpr::Similarity(Box::new(
                    crate::velesql::OrderByExpr::SimilarityBare,
                ))
            }
            crate::velesql::OrderByExpr::Similarity(sim) => {
                crate::velesql::ArithmeticExpr::Similarity(Box::new(
                    crate::velesql::OrderByExpr::Similarity(sim),
                ))
            }
            crate::velesql::OrderByExpr::Arithmetic(arith) => arith,
            crate::velesql::OrderByExpr::Aggregate(_) => {
                // Aggregates in LET are nonsensical; store as literal 0.
                crate::velesql::ArithmeticExpr::Literal(0.0)
            }
        }
    }

    pub(crate) fn parse_select_stmt(
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<SelectStatement, ParseError> {
        let mut stmt = SelectStmtBuilder::default();

        for inner_pair in pair.into_inner() {
            Self::dispatch_select_clause(inner_pair, &mut stmt)?;
        }

        Ok(stmt.build())
    }

    /// Dispatches a single clause within a SELECT statement to the appropriate parser.
    fn dispatch_select_clause(
        pair: pest::iterators::Pair<Rule>,
        stmt: &mut SelectStmtBuilder,
    ) -> Result<(), ParseError> {
        match pair.as_rule() {
            Rule::distinct_modifier => stmt.distinct = crate::velesql::DistinctMode::All,
            Rule::select_list => stmt.columns = Self::parse_select_list(pair)?,
            Rule::from_clause => Self::dispatch_from_clause(pair, stmt),
            Rule::join_clause => Self::dispatch_join_clause(pair, stmt)?,
            _ => Self::dispatch_optional_clause(pair, stmt)?,
        }
        Ok(())
    }

    fn dispatch_from_clause(pair: pest::iterators::Pair<Rule>, stmt: &mut SelectStmtBuilder) {
        let (table, aliases) = Self::parse_from_clause(pair);
        stmt.from = table;
        stmt.from_alias = aliases;
    }

    fn dispatch_join_clause(
        pair: pest::iterators::Pair<Rule>,
        stmt: &mut SelectStmtBuilder,
    ) -> Result<(), ParseError> {
        let join = Self::parse_join_clause(pair)?;
        // BUG-8: Collect JOIN aliases into from_alias so all
        // aliases visible in scope are available to the executor.
        if let Some(ref alias) = join.alias {
            stmt.from_alias.push(alias.clone());
        }
        stmt.joins.push(join);
        Ok(())
    }

    fn dispatch_optional_clause(
        pair: pest::iterators::Pair<Rule>,
        stmt: &mut SelectStmtBuilder,
    ) -> Result<(), ParseError> {
        match pair.as_rule() {
            Rule::where_clause => stmt.where_clause = Some(Self::parse_where_clause(pair)?),
            Rule::group_by_clause => stmt.group_by = Some(Self::parse_group_by_clause(pair)),
            Rule::having_clause => stmt.having = Some(Self::parse_having_clause(pair)?),
            Rule::order_by_clause => stmt.order_by = Some(Self::parse_order_by_clause(pair)?),
            _ => Self::dispatch_trailing_clause(pair, stmt)?,
        }
        Ok(())
    }

    /// Dispatches the trailing optional clauses (LIMIT, OFFSET, WITH, USING
    /// FUSION). Split out of `dispatch_optional_clause` to keep both functions
    /// within the cyclomatic-complexity budget.
    fn dispatch_trailing_clause(
        pair: pest::iterators::Pair<Rule>,
        stmt: &mut SelectStmtBuilder,
    ) -> Result<(), ParseError> {
        match pair.as_rule() {
            Rule::limit_clause => stmt.limit = Some(Self::parse_limit_clause(pair)?),
            Rule::offset_clause => stmt.offset = Some(Self::parse_offset_clause(pair)?),
            Rule::with_clause => stmt.with_clause = Some(Self::parse_with_clause(pair)?),
            Rule::using_fusion_clause => {
                stmt.fusion_clause = Some(Self::parse_using_fusion_clause(pair)?);
            }
            _ => {}
        }
        Ok(())
    }
}

/// Builder accumulator for `SelectStatement` fields during parsing.
struct SelectStmtBuilder {
    distinct: crate::velesql::DistinctMode,
    columns: SelectColumns,
    from: String,
    from_alias: Vec<String>,
    joins: Vec<crate::velesql::JoinClause>,
    where_clause: Option<crate::velesql::Condition>,
    order_by: Option<Vec<crate::velesql::SelectOrderBy>>,
    limit: Option<u64>,
    offset: Option<u64>,
    with_clause: Option<crate::velesql::WithClause>,
    group_by: Option<crate::velesql::GroupByClause>,
    having: Option<crate::velesql::HavingClause>,
    fusion_clause: Option<crate::velesql::FusionClause>,
}

impl Default for SelectStmtBuilder {
    fn default() -> Self {
        Self {
            distinct: crate::velesql::DistinctMode::None,
            columns: SelectColumns::All,
            from: String::new(),
            from_alias: Vec::new(),
            joins: Vec::new(),
            where_clause: None,
            order_by: None,
            limit: None,
            offset: None,
            with_clause: None,
            group_by: None,
            having: None,
            fusion_clause: None,
        }
    }
}

impl SelectStmtBuilder {
    fn build(self) -> SelectStatement {
        SelectStatement {
            distinct: self.distinct,
            columns: self.columns,
            from: self.from,
            from_alias: self.from_alias,
            joins: self.joins,
            where_clause: self.where_clause,
            order_by: self.order_by,
            limit: self.limit,
            offset: self.offset,
            with_clause: self.with_clause,
            group_by: self.group_by,
            having: self.having,
            fusion_clause: self.fusion_clause,
        }
    }
}
