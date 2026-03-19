//! Query validation for VelesQL (EPIC-044 US-007).

use super::ast::{Condition, OrderByExpr, Query, SelectColumns};
use super::error::{ParseError, ParseErrorKind};
use std::fmt;

const DEFAULT_MAX_QUERY_LENGTH: usize = 16_384;
const DEFAULT_MAX_AST_DEPTH: usize = 64;
const DEFAULT_MAX_LIKE_ILIKE_TERMS: usize = 8;
const DEFAULT_MAX_GRAPH_EXPANSION: u32 = 32;

#[derive(Debug, Clone, PartialEq)]
/// Validation error returned by parse-time semantic checks.
pub struct ValidationError {
    /// Machine-readable validation error kind.
    pub kind: ValidationErrorKind,
    /// Optional byte position in the input query.
    pub position: Option<usize>,
    /// Fragment of input associated with the failure.
    pub fragment: String,
    /// Human-readable remediation hint.
    pub suggestion: String,
}

impl ValidationError {
    #[must_use]
    /// Constructs a new validation error value.
    pub fn new(
        kind: ValidationErrorKind,
        position: Option<usize>,
        fragment: impl Into<String>,
        suggestion: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            position,
            fragment: fragment.into(),
            suggestion: suggestion.into(),
        }
    }

    #[must_use]
    /// Builds an error for unsupported multiple similarity predicates in OR branches.
    pub fn multiple_similarity(fragment: impl Into<String>) -> Self {
        Self::new(
            ValidationErrorKind::MultipleSimilarity,
            None,
            fragment,
            "Use AND instead of OR with similarity(), or split into separate queries",
        )
    }

    #[must_use]
    /// Builds an error for OR usage with similarity in strict modes.
    pub fn similarity_with_or(fragment: impl Into<String>) -> Self {
        Self::new(
            ValidationErrorKind::SimilarityWithOr,
            None,
            fragment,
            "Use AND instead of OR with similarity(), or split into separate queries",
        )
    }

    #[must_use]
    /// Builds an error for NOT similarity in strict modes.
    pub fn not_similarity(fragment: impl Into<String>) -> Self {
        Self::new(
            ValidationErrorKind::NotSimilarity,
            None,
            fragment,
            "NOT similarity() requires full scan. Add LIMIT clause to bound the scan",
        )
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(pos) = self.position {
            write!(
                f,
                "[{}] {} at position {}: {}",
                self.kind.code(),
                self.kind.message(),
                pos,
                self.suggestion
            )
        } else {
            write!(
                f,
                "[{}] {}: {}",
                self.kind.code(),
                self.kind.message(),
                self.suggestion
            )
        }
    }
}

impl std::error::Error for ValidationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Validation error categories.
pub enum ValidationErrorKind {
    /// Multiple similarity predicates combined in unsupported patterns.
    MultipleSimilarity,
    /// OR usage with similarity where strict mode forbids it.
    SimilarityWithOr,
    /// NOT similarity usage where strict mode forbids it.
    NotSimilarity,
    /// Reserved keyword misuse.
    ReservedKeyword,
    /// Invalid string escaping.
    StringEscaping,
    /// similarity() in SELECT/ORDER BY without a vector search context.
    SimilarityWithoutContext,
    /// Qualified wildcard alias not declared in FROM/JOIN.
    UndeclaredAlias,
}

impl ValidationErrorKind {
    #[must_use]
    /// Stable error code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MultipleSimilarity => "V001",
            Self::SimilarityWithOr => "V002",
            Self::NotSimilarity => "V003",
            Self::ReservedKeyword => "V004",
            Self::StringEscaping => "V005",
            Self::SimilarityWithoutContext => "V006",
            Self::UndeclaredAlias => "V007",
        }
    }

    #[must_use]
    /// Human-readable default message.
    pub const fn message(&self) -> &'static str {
        match self {
            Self::MultipleSimilarity => "Multiple similarity() conditions not supported",
            Self::SimilarityWithOr => "OR operator not supported with similarity()",
            Self::NotSimilarity => "NOT similarity() requires full scan",
            Self::ReservedKeyword => "Reserved keyword requires escaping",
            Self::StringEscaping => "Invalid string escaping",
            Self::SimilarityWithoutContext => {
                "similarity() requires a vector search context (NEAR or similarity() in WHERE)"
            }
            Self::UndeclaredAlias => "Qualified wildcard references an undeclared alias",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// Validation and complexity limits applied at parse-time.
pub struct ValidationConfig {
    /// If true, keeps strict semantics for NOT similarity checks.
    pub strict_not_similarity: bool,
    /// Maximum accepted raw query length in bytes.
    pub max_query_length: usize,
    /// Maximum boolean-condition AST depth.
    pub max_ast_depth: usize,
    /// Maximum number of LIKE/ILIKE terms allowed in WHERE trees.
    pub max_like_ilike_terms: usize,
    /// Maximum graph expansion hops inferred from MATCH relationship ranges.
    pub max_graph_expansion: u32,
}

impl Default for ValidationConfig {
    fn default() -> Self {
        Self {
            strict_not_similarity: true,
            max_query_length: DEFAULT_MAX_QUERY_LENGTH,
            max_ast_depth: DEFAULT_MAX_AST_DEPTH,
            max_like_ilike_terms: DEFAULT_MAX_LIKE_ILIKE_TERMS,
            max_graph_expansion: DEFAULT_MAX_GRAPH_EXPANSION,
        }
    }
}

impl ValidationConfig {
    #[must_use]
    /// Strict profile used in production by default.
    pub fn strict() -> Self {
        Self::default()
    }

    #[must_use]
    /// Lenient profile disabling strict NOT similarity checks.
    pub fn lenient() -> Self {
        Self {
            strict_not_similarity: false,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Computed complexity features for a query.
pub struct ComplexityStats {
    /// Maximum condition AST depth.
    pub ast_depth: usize,
    /// Number of LIKE/ILIKE terms in all validated WHERE branches.
    pub like_ilike_terms: usize,
    /// Maximum hop upper bound extracted from MATCH range expressions.
    pub max_graph_hops: u32,
}

/// Stateless validator for semantic and complexity checks.
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
        if query.is_dml_query() {
            return Ok(());
        }

        if let Some(ref condition) = query.select.where_clause {
            Self::validate_condition(condition, query.select.limit, config)?;
        }
        if let Some(ref compound) = query.compound {
            if let Some(ref condition) = compound.right.where_clause {
                Self::validate_condition(condition, compound.right.limit, config)?;
            }
        }

        Self::validate_similarity_context(&query.select)?;
        Self::validate_qualified_wildcards(&query.select)?;

        if let Some(ref compound) = query.compound {
            Self::validate_similarity_context(&compound.right)?;
            Self::validate_qualified_wildcards(&compound.right)?;
        }

        Ok(())
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

        if !has_score_context {
            if let Some(ref order_by) = stmt.order_by {
                let uses_bare = order_by
                    .iter()
                    .any(|ob| ob.expr == OrderByExpr::SimilarityBare);
                if uses_bare {
                    return Err(ValidationError::new(
                        ValidationErrorKind::SimilarityWithoutContext,
                        None,
                        "ORDER BY similarity()",
                        "Add a vector NEAR or similarity() predicate in WHERE to provide a score context",
                    ));
                }
            }
        }

        Ok(())
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
            return Err(ParseError::new(
                ParseErrorKind::ComplexityLimit,
                config.max_query_length,
                raw_query.chars().take(128).collect::<String>(),
                format!(
                    "Query length exceeded: max={}, actual={}",
                    config.max_query_length,
                    raw_query.len()
                ),
            ));
        }

        let stats = Self::analyze_query_complexity(query);
        if stats.ast_depth > config.max_ast_depth {
            return Err(ParseError::new(
                ParseErrorKind::ComplexityLimit,
                0,
                "WHERE",
                format!(
                    "AST depth exceeded: max={}, actual={}",
                    config.max_ast_depth, stats.ast_depth
                ),
            ));
        }
        if stats.like_ilike_terms > config.max_like_ilike_terms {
            return Err(ParseError::new(
                ParseErrorKind::ComplexityLimit,
                0,
                "LIKE/ILIKE",
                format!(
                    "LIKE/ILIKE budget exceeded: max={}, actual={}",
                    config.max_like_ilike_terms, stats.like_ilike_terms
                ),
            ));
        }
        if stats.max_graph_hops > config.max_graph_expansion {
            return Err(ParseError::new(
                ParseErrorKind::ComplexityLimit,
                0,
                "MATCH",
                format!(
                    "Graph expansion exceeded: max={}, actual={}",
                    config.max_graph_expansion, stats.max_graph_hops
                ),
            ));
        }

        Ok(())
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
            if let Some(ref condition) = compound.right.where_clause {
                let (depth, like_count) = Self::analyze_condition(condition);
                stats.ast_depth = stats.ast_depth.max(depth);
                stats.like_ilike_terms += like_count;
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
}
