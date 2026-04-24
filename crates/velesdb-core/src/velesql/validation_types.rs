//! Type definitions for query validation (EPIC-044 US-007).
//!
//! Separated from `validation.rs` to keep each file under the 500 NLOC limit.

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

#[non_exhaustive]
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
    /// Parameterized `similarity(field, $vec)` inside arithmetic is not yet supported.
    UnsupportedArithmeticSimilarity,
    /// LET bindings used with DDL or DML statements (nonsensical).
    InvalidLetBinding,
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
            Self::UnsupportedArithmeticSimilarity => "V008",
            Self::InvalidLetBinding => "V009",
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
            Self::UnsupportedArithmeticSimilarity => {
                "Parameterized similarity(field, $vec) inside arithmetic expressions \
                 is not yet supported"
            }
            Self::InvalidLetBinding => "LET bindings are not supported with DDL or DML statements",
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
