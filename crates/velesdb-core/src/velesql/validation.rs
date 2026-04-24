//! Query validation for VelesQL (EPIC-044 US-007).
//!
//! Type definitions (`ValidationError`, `ValidationErrorKind`, `ValidationConfig`,
//! `ComplexityStats`) live in `validation_types.rs` to keep each file under
//! the 500 NLOC limit.

use super::ast::{ArithmeticExpr, Condition, OrderByExpr, Query, SelectColumns};
use super::error::{ParseError, ParseErrorKind};

// Re-export types so that existing `use crate::velesql::validation::*` paths
// continue to work without changes.
pub use super::validation_types::{
    ComplexityStats, ValidationConfig, ValidationError, ValidationErrorKind,
};

/// Stateless validator for VelesQL semantic and complexity checks.
///
/// Performs validation passes on parsed [`Query`] ASTs:
/// - LET binding legality (DDL/DML/admin cannot use LET)
/// - Similarity context (similarity() requires a score-producing WHERE clause)
/// - Qualified wildcard alias resolution
/// - Compound query validation across UNION/INTERSECT/EXCEPT operands
/// - Complexity budget enforcement (AST depth, LIKE/ILIKE terms, graph hops)
pub struct QueryValidator;

impl QueryValidator {
    /// Validates a query with default configuration.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if the query fails semantic validation.
    pub fn validate(query: &Query) -> Result<(), ValidationError> {
        Self::validate_with_config(query, &ValidationConfig::default())
    }

    /// Validates a query with custom semantic configuration.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if the query fails semantic validation.
    pub fn validate_with_config(
        query: &Query,
        config: &ValidationConfig,
    ) -> Result<(), ValidationError> {
        // Non-SELECT statements: only check LET bindings.
        if !requires_select_validation(query) {
            return reject_let_on_non_select(query);
        }

        Self::validate_select(&query.select, config)?;

        if let Some(ref compound) = query.compound {
            for (_, right_select) in &compound.operations {
                Self::validate_select(right_select, config)?;
            }
        }

        Ok(())
    }

    /// Validates a single SELECT statement (main or compound operand).
    fn validate_select(
        stmt: &super::ast::SelectStatement,
        config: &ValidationConfig,
    ) -> Result<(), ValidationError> {
        if let Some(ref condition) = stmt.where_clause {
            Self::validate_condition(condition, stmt.limit, config)?;
        }
        Self::validate_similarity_context(stmt)?;
        Self::validate_qualified_wildcards(stmt)?;
        Self::validate_vector_group_by(stmt)
    }

    /// Validates that `similarity()` in SELECT or ORDER BY has a score context.
    fn validate_similarity_context(
        stmt: &super::ast::SelectStatement,
    ) -> Result<(), ValidationError> {
        let has_score_context = stmt
            .where_clause
            .as_ref()
            .is_some_and(Self::has_score_producing_condition);

        if !has_score_context && Self::select_uses_similarity(&stmt.columns) {
            return Err(ValidationError::new(
                ValidationErrorKind::SimilarityWithoutContext,
                None,
                "similarity()",
                "Add a vector NEAR or similarity() predicate in WHERE to provide a score context",
            ));
        }

        if let Some(ref order_by) = stmt.order_by {
            for ob in order_by {
                Self::validate_order_by_expr(&ob.expr, has_score_context)?;
            }
        }

        Ok(())
    }

    /// Validates a single ORDER BY expression for similarity context issues.
    fn validate_order_by_expr(
        expr: &OrderByExpr,
        has_score_context: bool,
    ) -> Result<(), ValidationError> {
        match expr {
            OrderByExpr::SimilarityBare if !has_score_context => Err(ValidationError::new(
                ValidationErrorKind::SimilarityWithoutContext,
                None,
                "ORDER BY similarity()",
                "Add a vector NEAR or similarity() predicate in WHERE to provide a score context",
            )),
            OrderByExpr::Arithmetic(arith) => {
                Self::validate_arithmetic_similarity(arith, has_score_context)
            }
            _ => Ok(()),
        }
    }

    /// Recursively validates similarity() usage inside arithmetic expressions.
    fn validate_arithmetic_similarity(
        expr: &ArithmeticExpr,
        has_score_context: bool,
    ) -> Result<(), ValidationError> {
        match expr {
            ArithmeticExpr::Similarity(inner) => match inner.as_ref() {
                OrderByExpr::Similarity(_) => Err(ValidationError::new(
                    ValidationErrorKind::UnsupportedArithmeticSimilarity,
                    None,
                    "similarity(field, $vec) in arithmetic",
                    "Use bare similarity() instead; parameterized similarity inside arithmetic is not yet supported",
                )),
                OrderByExpr::SimilarityBare if !has_score_context => Err(ValidationError::new(
                    ValidationErrorKind::SimilarityWithoutContext,
                    None,
                    "similarity() in arithmetic",
                    "Add a vector NEAR or similarity() predicate in WHERE to provide a score context",
                )),
                _ => Ok(()),
            },
            ArithmeticExpr::BinaryOp { left, right, .. } => {
                Self::validate_arithmetic_similarity(left, has_score_context)?;
                Self::validate_arithmetic_similarity(right, has_score_context)
            }
            ArithmeticExpr::Literal(_) | ArithmeticExpr::Variable(_) => Ok(()),
        }
    }

    /// Returns true if `SelectColumns` references `similarity()`.
    fn select_uses_similarity(columns: &SelectColumns) -> bool {
        match columns {
            SelectColumns::SimilarityScore(_) => true,
            SelectColumns::Mixed {
                similarity_scores, ..
            } => !similarity_scores.is_empty(),
            _ => false,
        }
    }

    /// Validates that qualified wildcard aliases are declared in FROM/JOIN.
    fn validate_qualified_wildcards(
        stmt: &super::ast::SelectStatement,
    ) -> Result<(), ValidationError> {
        let aliases = &stmt.from_alias;
        let from_name = &stmt.from;

        let check_alias = |alias: &str| -> Result<(), ValidationError> {
            let is_declared = aliases.iter().any(|a| a == alias) || alias == from_name;
            if !is_declared {
                return Err(ValidationError::new(
                    ValidationErrorKind::UndeclaredAlias,
                    None,
                    format!("{alias}.*"),
                    format!(
                        "Alias '{alias}' is not declared in FROM or JOIN. Use FROM ... AS {alias}"
                    ),
                ));
            }
            Ok(())
        };

        match &stmt.columns {
            SelectColumns::QualifiedWildcard(alias) => check_alias(alias)?,
            SelectColumns::Mixed {
                qualified_wildcards,
                ..
            } => {
                for alias in qualified_wildcards {
                    check_alias(alias)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Enforces complexity budgets and returns parse errors on overflow.
    ///
    /// # Errors
    ///
    /// Returns `ParseError` if the query exceeds configured complexity limits.
    pub fn enforce_query_complexity(
        query: &Query,
        raw_query: &str,
        config: &ValidationConfig,
    ) -> Result<(), ParseError> {
        if raw_query.len() > config.max_query_length {
            return Err(Self::complexity_error(
                config.max_query_length,
                raw_query.chars().take(128).collect::<String>(),
                "Query length",
                config.max_query_length,
                raw_query.len(),
            ));
        }

        let stats = Self::analyze_query_complexity(query);
        Self::check_limit(stats.ast_depth, config.max_ast_depth, "AST depth", "WHERE")?;
        Self::check_limit(
            stats.like_ilike_terms,
            config.max_like_ilike_terms,
            "LIKE/ILIKE budget",
            "LIKE/ILIKE",
        )?;
        Self::check_limit_u32(
            stats.max_graph_hops,
            config.max_graph_expansion,
            "Graph expansion",
            "MATCH",
        )?;

        Ok(())
    }

    /// Returns a complexity-limit error when `actual > max`.
    fn check_limit(
        actual: usize,
        max: usize,
        label: &str,
        context: &str,
    ) -> Result<(), ParseError> {
        if actual > max {
            return Err(Self::complexity_error(0, context, label, max, actual));
        }
        Ok(())
    }

    /// Returns a complexity-limit error when `actual > max` (u32 variant).
    fn check_limit_u32(
        actual: u32,
        max: u32,
        label: &str,
        context: &str,
    ) -> Result<(), ParseError> {
        if actual > max {
            return Err(Self::complexity_error(
                0,
                context,
                label,
                max as usize,
                actual as usize,
            ));
        }
        Ok(())
    }

    /// Builds a [`ParseError`] for a complexity-limit violation.
    fn complexity_error(
        position: usize,
        context: impl Into<String>,
        label: &str,
        max: usize,
        actual: usize,
    ) -> ParseError {
        ParseError::new(
            ParseErrorKind::ComplexityLimit,
            position,
            context,
            format!("{label} exceeded: max={max}, actual={actual}"),
        )
    }

    #[must_use]
    /// Extracts complexity statistics from a parsed query.
    pub fn analyze_query_complexity(query: &Query) -> ComplexityStats {
        let mut stats = ComplexityStats {
            ast_depth: 0,
            like_ilike_terms: 0,
            max_graph_hops: 0,
        };

        if let Some(ref condition) = query.select.where_clause {
            let (depth, like_count) = Self::analyze_condition(condition);
            stats.ast_depth = stats.ast_depth.max(depth);
            stats.like_ilike_terms += like_count;
        }

        if let Some(ref compound) = query.compound {
            for (_, right_select) in &compound.operations {
                if let Some(ref condition) = right_select.where_clause {
                    let (depth, like_count) = Self::analyze_condition(condition);
                    stats.ast_depth = stats.ast_depth.max(depth);
                    stats.like_ilike_terms += like_count;
                }
            }
        }

        if let Some(ref m) = query.match_clause {
            for rel in m.patterns.iter().flat_map(|p| p.relationships.iter()) {
                if let Some((_, max)) = rel.range {
                    stats.max_graph_hops = stats.max_graph_hops.max(max);
                }
            }
        }

        stats
    }

    fn validate_condition(
        condition: &Condition,
        _limit: Option<u64>,
        _config: &ValidationConfig,
    ) -> Result<(), ValidationError> {
        let similarity_count = Self::count_similarity_conditions(condition);
        if similarity_count > 1 && Self::has_multiple_similarity_in_or(condition) {
            return Err(ValidationError::multiple_similarity(
                "Multiple similarity() in OR are not supported. Use AND instead.",
            ));
        }
        Ok(())
    }

    fn analyze_condition(condition: &Condition) -> (usize, usize) {
        match condition {
            Condition::Like(_) => (1, 1),
            Condition::And(l, r) | Condition::Or(l, r) => {
                let (ld, ll) = Self::analyze_condition(l);
                let (rd, rl) = Self::analyze_condition(r);
                (1 + ld.max(rd), ll + rl)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                let (d, l) = Self::analyze_condition(inner);
                (1 + d, l)
            }
            _ => (1, 0),
        }
    }

    /// Returns true if the condition contains any score-producing search
    /// (vector, similarity, fused, or sparse).
    fn has_score_producing_condition(condition: &Condition) -> bool {
        match condition {
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_)
            | Condition::SparseVectorSearch(_) => true,
            Condition::And(l, r) | Condition::Or(l, r) => {
                Self::has_score_producing_condition(l) || Self::has_score_producing_condition(r)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::has_score_producing_condition(inner)
            }
            _ => false,
        }
    }

    pub(crate) fn count_similarity_conditions(condition: &Condition) -> usize {
        match condition {
            Condition::Similarity(_)
            | Condition::VectorSearch(_)
            | Condition::VectorFusedSearch(_) => 1,
            Condition::And(l, r) | Condition::Or(l, r) => {
                Self::count_similarity_conditions(l) + Self::count_similarity_conditions(r)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::count_similarity_conditions(inner)
            }
            _ => 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn contains_similarity(condition: &Condition) -> bool {
        Self::count_similarity_conditions(condition) > 0
    }

    #[cfg(test)]
    pub(crate) fn has_not_similarity(condition: &Condition) -> bool {
        match condition {
            Condition::Not(inner) => Self::contains_similarity(inner),
            Condition::And(l, r) | Condition::Or(l, r) => {
                Self::has_not_similarity(l) || Self::has_not_similarity(r)
            }
            Condition::Group(inner) => Self::has_not_similarity(inner),
            _ => false,
        }
    }

    fn has_multiple_similarity_in_or(condition: &Condition) -> bool {
        match condition {
            Condition::Or(l, r) => {
                Self::count_similarity_conditions(l) > 0 && Self::count_similarity_conditions(r) > 0
                    || Self::has_multiple_similarity_in_or(l)
                    || Self::has_multiple_similarity_in_or(r)
            }
            Condition::And(l, r) => {
                Self::has_multiple_similarity_in_or(l) || Self::has_multiple_similarity_in_or(r)
            }
            Condition::Not(inner) | Condition::Group(inner) => {
                Self::has_multiple_similarity_in_or(inner)
            }
            _ => false,
        }
    }

    /// Validates vector-search GROUP BY semantic constraints.
    ///
    /// - `FIRST(column)` requires GROUP BY
    /// - `MAX(score)` / `AVG(score)` requires vector NEAR in WHERE
    fn validate_vector_group_by(stmt: &super::ast::SelectStatement) -> Result<(), ValidationError> {
        let aggregations = Self::collect_aggregations(&stmt.columns);
        let has_group_by = stmt.group_by.is_some();
        let has_vector_near = stmt
            .where_clause
            .as_ref()
            .is_some_and(super::ast::condition::Condition::has_vector_search);

        for agg in &aggregations {
            Self::validate_single_aggregate(agg, has_group_by, has_vector_near)?;
        }
        Ok(())
    }

    /// Validates a single aggregate function for vector GROUP BY constraints.
    fn validate_single_aggregate(
        agg: &super::ast::AggregateFunction,
        has_group_by: bool,
        has_vector_near: bool,
    ) -> Result<(), ValidationError> {
        if matches!(agg.function_type, super::ast::AggregateType::First) && !has_group_by {
            return Err(ValidationError::new(
                ValidationErrorKind::InvalidLetBinding,
                None,
                "FIRST()",
                "FIRST() aggregate function requires a GROUP BY clause",
            ));
        }
        // MAX(score)/AVG(score) without NEAR is only an error when GROUP BY is present,
        // because without GROUP BY, "score" is treated as a regular payload field.
        if has_group_by && Self::is_score_column(&agg.argument) && !has_vector_near {
            let fn_name = format!("{:?}(score)", agg.function_type);
            let msg = format!(
                "{}(score) requires a vector NEAR search in the WHERE clause",
                format!("{:?}", agg.function_type).to_uppercase()
            );
            return Err(ValidationError::new(
                ValidationErrorKind::SimilarityWithoutContext,
                None,
                fn_name,
                msg,
            ));
        }
        Ok(())
    }

    /// Returns `true` if the argument references the score pseudo-column.
    fn is_score_column(arg: &super::ast::AggregateArg) -> bool {
        matches!(arg, super::ast::AggregateArg::Score)
            || matches!(arg, super::ast::AggregateArg::Column(col) if col.eq_ignore_ascii_case("score"))
    }

    /// Collects aggregate functions from `SelectColumns`.
    fn collect_aggregations(columns: &SelectColumns) -> Vec<super::ast::AggregateFunction> {
        match columns {
            SelectColumns::Aggregations(aggs) => aggs.clone(),
            SelectColumns::Mixed { aggregations, .. } => aggregations.clone(),
            _ => Vec::new(),
        }
    }
}

/// Returns `true` if the query requires SELECT-specific validation passes.
///
/// DDL, DML, introspection, admin, and TRAIN statements bypass SELECT
/// validation (no FROM clause, no similarity conditions, etc.).
fn requires_select_validation(query: &Query) -> bool {
    !query.is_ddl_query()
        && !query.is_dml_query()
        && !query.is_train()
        && !query.is_introspection_query()
        && !query.is_admin_query()
}

/// Returns `Ok(())` if there are no LET bindings, otherwise rejects.
fn reject_let_on_non_select(query: &Query) -> Result<(), ValidationError> {
    if query.let_bindings.is_empty() {
        Ok(())
    } else {
        Err(ValidationError::new(
            ValidationErrorKind::InvalidLetBinding,
            None,
            "LET clause",
            "LET bindings are not supported with DDL, DML, introspection, or admin statements",
        ))
    }
}
