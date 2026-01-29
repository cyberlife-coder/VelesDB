//! Tests for quantization hints in WITH clause (EPIC-055 US-005).
//!
//! Covers:
//! - WITH (quantization = 'dual') parsing
//! - WITH (quantization = 'f32') parsing
//! - WITH (quantization = 'int8') parsing
//! - WITH (quantization = 'auto') parsing
//! - WITH (oversampling = N) for dual-precision
//! - Invalid quantization values
//! - Combined options

use crate::velesql::{Parser, QuantizationMode, WithClause};

// =========================================================================
// TDD Tests for Quantization Hints Parsing
// =========================================================================

#[test]
fn test_parse_quantization_dual() {
    let sql =
        "SELECT * FROM docs WHERE similarity(embedding, $v) > 0.8 WITH (quantization = 'dual')";
    let result = Parser::parse(sql);
    assert!(
        result.is_ok(),
        "Failed to parse quantization=dual: {:?}",
        result.err()
    );

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    let mode = with_clause
        .get_quantization()
        .expect("quantization should be present");
    assert_eq!(mode, QuantizationMode::Dual);
}

#[test]
fn test_parse_quantization_f32() {
    let sql = "SELECT * FROM docs WITH (quantization = 'f32')";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let mode = query
        .select
        .with_clause
        .as_ref()
        .and_then(WithClause::get_quantization)
        .expect("quantization should be present");

    assert_eq!(mode, QuantizationMode::F32);
}

#[test]
fn test_parse_quantization_int8() {
    let sql = "SELECT * FROM docs WITH (quantization = 'int8')";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let mode = query
        .select
        .with_clause
        .as_ref()
        .and_then(WithClause::get_quantization)
        .expect("quantization should be present");

    assert_eq!(mode, QuantizationMode::Int8);
}

#[test]
fn test_parse_quantization_auto() {
    let sql = "SELECT * FROM docs WITH (quantization = 'auto')";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let mode = query
        .select
        .with_clause
        .as_ref()
        .and_then(WithClause::get_quantization)
        .expect("quantization should be present");

    assert_eq!(mode, QuantizationMode::Auto);
}

#[test]
fn test_parse_oversampling() {
    let sql = "SELECT * FROM docs WITH (quantization = 'dual', oversampling = 8)";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    let mode = with_clause
        .get_quantization()
        .expect("quantization present");
    assert_eq!(mode, QuantizationMode::Dual);

    let oversampling = with_clause
        .get_oversampling()
        .expect("oversampling present");
    assert_eq!(oversampling, 8);
}

#[test]
fn test_parse_quantization_combined_with_timeout() {
    let sql = "SELECT * FROM docs WITH (quantization = 'dual', timeout_ms = 5000)";
    let result = Parser::parse(sql);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let query = result.unwrap();
    let with_clause = query
        .select
        .with_clause
        .as_ref()
        .expect("WITH clause should be present");

    assert_eq!(with_clause.get_quantization(), Some(QuantizationMode::Dual));
    assert_eq!(with_clause.get_timeout_ms(), Some(5000));
}

#[test]
fn test_no_quantization_returns_none() {
    let sql = "SELECT * FROM docs WITH (timeout_ms = 1000)";
    let result = Parser::parse(sql);
    assert!(result.is_ok());

    let query = result.unwrap();
    let with_clause = query.select.with_clause.as_ref().unwrap();

    assert!(with_clause.get_quantization().is_none());
}

#[test]
fn test_quantization_mode_default() {
    let default = QuantizationMode::default();
    assert_eq!(default, QuantizationMode::Auto);
}

// =========================================================================
// TDD Tests for QuantizationMode enum
// =========================================================================

#[test]
fn test_quantization_mode_parse() {
    assert_eq!(
        QuantizationMode::parse("dual"),
        Some(QuantizationMode::Dual)
    );
    assert_eq!(QuantizationMode::parse("f32"), Some(QuantizationMode::F32));
    assert_eq!(
        QuantizationMode::parse("int8"),
        Some(QuantizationMode::Int8)
    );
    assert_eq!(
        QuantizationMode::parse("auto"),
        Some(QuantizationMode::Auto)
    );
    assert_eq!(
        QuantizationMode::parse("DUAL"),
        Some(QuantizationMode::Dual)
    );
    assert_eq!(QuantizationMode::parse("invalid"), None);
}

#[test]
fn test_quantization_mode_as_str() {
    assert_eq!(QuantizationMode::Dual.as_str(), "dual");
    assert_eq!(QuantizationMode::F32.as_str(), "f32");
    assert_eq!(QuantizationMode::Int8.as_str(), "int8");
    assert_eq!(QuantizationMode::Auto.as_str(), "auto");
}
