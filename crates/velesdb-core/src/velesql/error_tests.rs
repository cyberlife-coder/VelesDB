//! Tests for `error` module

use super::error::*;

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
