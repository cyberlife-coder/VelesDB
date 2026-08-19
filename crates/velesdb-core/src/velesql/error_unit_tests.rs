use super::*;

#[test]
fn test_parse_error_new() {
    let err = ParseError::new(ParseErrorKind::SyntaxError, 10, "SELECT", "Invalid syntax");
    assert_eq!(err.kind, ParseErrorKind::SyntaxError);
    assert_eq!(err.position, 10);
    assert_eq!(err.fragment, "SELECT");
    assert_eq!(err.message, "Invalid syntax");
}

#[test]
fn test_parse_error_syntax() {
    let err = ParseError::syntax(5, "FROM", "Expected table name");
    assert_eq!(err.kind, ParseErrorKind::SyntaxError);
    assert_eq!(err.position, 5);
}

#[test]
fn test_parse_error_unexpected_token() {
    let err = ParseError::unexpected_token(0, "xyz", "identifier");
    assert_eq!(err.kind, ParseErrorKind::UnexpectedToken);
    assert!(err.message.contains("Expected identifier"));
}

#[test]
fn test_parse_error_unknown_column() {
    let err = ParseError::unknown_column("missing_col");
    assert_eq!(err.kind, ParseErrorKind::UnknownColumn);
    assert!(err.message.contains("missing_col"));
}

#[test]
fn test_parse_error_missing_parameter() {
    let err = ParseError::missing_parameter("vector");
    assert_eq!(err.kind, ParseErrorKind::MissingParameter);
    assert!(err.message.contains("$vector"));
}

#[test]
fn test_parse_error_display() {
    let err = ParseError::syntax(10, "bad", "Syntax error");
    let display = format!("{err}");
    assert!(display.contains("E001"));
    assert!(display.contains("Syntax error"));
    assert!(display.contains("10"));
}

#[test]
fn test_parse_error_kind_codes() {
    assert_eq!(ParseErrorKind::SyntaxError.code(), "E001");
    assert_eq!(ParseErrorKind::UnexpectedToken.code(), "E001");
    assert_eq!(ParseErrorKind::UnknownColumn.code(), "E002");
    assert_eq!(ParseErrorKind::CollectionNotFound.code(), "E003");
    assert_eq!(ParseErrorKind::DimensionMismatch.code(), "E004");
    assert_eq!(ParseErrorKind::MissingParameter.code(), "E005");
    assert_eq!(ParseErrorKind::TypeMismatch.code(), "E006");
}

#[test]
fn test_parse_error_is_std_error() {
    let err = ParseError::syntax(0, "", "test");
    let _: &dyn std::error::Error = &err;
}
