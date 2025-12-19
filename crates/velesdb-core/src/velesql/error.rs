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

    /// Creates a missing parameter error.
    #[must_use]
    pub fn missing_parameter(param: impl Into<String>) -> Self {
        let p = param.into();
        Self::new(
            ParseErrorKind::MissingParameter,
            0,
            p.clone(),
            format!("Missing parameter '${p}'"),
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
    /// Missing parameter (E005).
    MissingParameter,
    /// Type mismatch (E006).
    TypeMismatch,
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
            Self::MissingParameter => "E005",
            Self::TypeMismatch => "E006",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syntax_error() {
        let err = ParseError::syntax(15, "FORM", "Expected FROM");
        assert_eq!(err.kind, ParseErrorKind::SyntaxError);
        assert_eq!(err.position, 15);
        assert_eq!(err.fragment, "FORM");
        assert!(err.message.contains("FROM"));
    }

    #[test]
    fn test_unexpected_token() {
        let err = ParseError::unexpected_token(10, "123", "identifier");
        assert_eq!(err.kind, ParseErrorKind::UnexpectedToken);
        assert!(err.message.contains("identifier"));
    }

    #[test]
    fn test_unknown_column() {
        let err = ParseError::unknown_column("nonexistent");
        assert_eq!(err.kind, ParseErrorKind::UnknownColumn);
        assert!(err.message.contains("nonexistent"));
    }

    #[test]
    fn test_missing_parameter() {
        let err = ParseError::missing_parameter("query_vector");
        assert_eq!(err.kind, ParseErrorKind::MissingParameter);
        assert!(err.message.contains("query_vector"));
    }

    #[test]
    fn test_error_codes() {
        assert_eq!(ParseErrorKind::SyntaxError.code(), "E001");
        assert_eq!(ParseErrorKind::UnknownColumn.code(), "E002");
        assert_eq!(ParseErrorKind::CollectionNotFound.code(), "E003");
        assert_eq!(ParseErrorKind::DimensionMismatch.code(), "E004");
        assert_eq!(ParseErrorKind::MissingParameter.code(), "E005");
        assert_eq!(ParseErrorKind::TypeMismatch.code(), "E006");
    }

    #[test]
    fn test_error_display() {
        let err = ParseError::syntax(15, "FORM", "Expected FROM keyword");
        let display = format!("{err}");
        assert!(display.contains("E001"));
        assert!(display.contains("15"));
        assert!(display.contains("FROM"));
    }
}
