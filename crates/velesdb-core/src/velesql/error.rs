//! Error types for `VelesQL` parsing.

use std::fmt;

/// Error that occurred during parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    /// Kind of error.
    pub kind: ParseErrorKind,
    /// Position in the input where the error occurred.
    pub position: usize,
    /// The problematic input fragment.
    pub fragment: String,
    /// Human-readable message.
    pub message: String,
}

impl ParseError {
    /// Creates a new parse error.
    #[must_use]
    pub fn new(
        kind: ParseErrorKind,
        position: usize,
        fragment: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            position,
            fragment: fragment.into(),
            message: message.into(),
        }
    }

    /// Creates a syntax error.
    #[must_use]
    pub fn syntax(
        position: usize,
        fragment: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(ParseErrorKind::SyntaxError, position, fragment, message)
    }

    /// Creates an unexpected token error.
    #[must_use]
    pub fn unexpected_token(position: usize, fragment: impl Into<String>, expected: &str) -> Self {
        Self::new(
            ParseErrorKind::UnexpectedToken,
            position,
            fragment,
            format!("Expected {expected}"),
        )
    }

    /// Creates an unknown column error.
    #[must_use]
    pub fn unknown_column(column: impl Into<String>) -> Self {
        let col = column.into();
        Self::new(
            ParseErrorKind::UnknownColumn,
            0,
            col.clone(),
            format!("Unknown column '{col}'"),
        )
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {} at position {}",
            self.kind.code(),
            self.message,
            self.position
        )
    }
}

impl std::error::Error for ParseError {}

/// Kind of parse error.
///
/// A `$name` the caller did not bind, or bound to a value its position cannot
/// take, is not a parse error — the parser never sees the parameters — but
/// an execution error, [`crate::Error::QueryParameter`] (`VELES-038`).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// Syntax error (E001).
    SyntaxError,
    /// Unexpected token (E001).
    UnexpectedToken,
    /// Unknown column (E002).
    UnknownColumn,
    /// Collection not found (E003).
    CollectionNotFound,
    /// Vector dimension mismatch (E004).
    DimensionMismatch,
    /// Type mismatch (E006).
    TypeMismatch,
    /// Query complexity guardrail exceeded (E007).
    ComplexityLimit,
}

impl ParseErrorKind {
    /// Returns the error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::SyntaxError | Self::UnexpectedToken => "E001",
            Self::UnknownColumn => "E002",
            Self::CollectionNotFound => "E003",
            Self::DimensionMismatch => "E004",
            Self::TypeMismatch => "E006",
            Self::ComplexityLimit => "E007",
        }
    }
}

#[cfg(test)]
#[path = "error_unit_tests.rs"]
mod tests;
