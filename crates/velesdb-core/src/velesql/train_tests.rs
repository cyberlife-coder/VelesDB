//! Tests for TRAIN QUANTIZER statement parsing.

use crate::velesql::{Parser, WithValue};

#[test]
fn test_train_quantizer_basic() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8, k=256)").unwrap();
    assert!(query.is_train());
    let train = query.train.as_ref().unwrap();
    assert_eq!(train.collection, "my_collection");
    assert_eq!(train.params.get("m"), Some(&WithValue::Integer(8)));
    assert_eq!(train.params.get("k"), Some(&WithValue::Integer(256)));
}

#[test]
fn test_train_quantizer_single_param() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8)").unwrap();
    assert!(query.is_train());
    let train = query.train.as_ref().unwrap();
    assert_eq!(train.collection, "my_collection");
    assert_eq!(train.params.len(), 1);
    assert_eq!(train.params.get("m"), Some(&WithValue::Integer(8)));
}

#[test]
fn test_train_quantizer_all_params() {
    let query = Parser::parse(
        "TRAIN QUANTIZER ON my_collection WITH (m=8, k=256, type=opq, oversampling=4)",
    )
    .unwrap();
    assert!(query.is_train());
    let train = query.train.as_ref().unwrap();
    assert_eq!(train.collection, "my_collection");
    assert_eq!(train.params.len(), 4);
    assert_eq!(train.params.get("m"), Some(&WithValue::Integer(8)));
    assert_eq!(train.params.get("k"), Some(&WithValue::Integer(256)));
    assert_eq!(
        train.params.get("type"),
        Some(&WithValue::Identifier("opq".to_string()))
    );
    assert_eq!(
        train.params.get("oversampling"),
        Some(&WithValue::Integer(4))
    );
}

#[test]
fn test_train_quantizer_boolean_param() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8, force=true)").unwrap();
    assert!(query.is_train());
    let train = query.train.as_ref().unwrap();
    assert_eq!(train.params.get("force"), Some(&WithValue::Boolean(true)));
}

#[test]
fn test_train_quantizer_missing_on_keyword() {
    let result = Parser::parse("TRAIN QUANTIZER my_collection WITH (m=8)");
    assert!(result.is_err());
}

#[test]
fn test_train_quantizer_missing_quantizer_keyword() {
    let result = Parser::parse("TRAIN my_collection WITH (m=8)");
    assert!(result.is_err());
}

#[test]
fn test_is_train_true_for_train_statement() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8)").unwrap();
    assert!(query.is_train());
}

#[test]
fn test_is_train_false_for_select() {
    let query = Parser::parse("SELECT * FROM docs LIMIT 10").unwrap();
    assert!(!query.is_train());
}

#[test]
fn test_train_collection_name() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8)").unwrap();
    let train = query.train.as_ref().unwrap();
    assert_eq!(train.collection, "my_collection");
}

#[test]
fn test_train_param_m_integer() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8)").unwrap();
    let train = query.train.as_ref().unwrap();
    assert_eq!(train.params.get("m"), Some(&WithValue::Integer(8)));
}

#[test]
fn test_train_quantizer_with_semicolon() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8, k=256);").unwrap();
    assert!(query.is_train());
}

#[test]
fn test_train_quantizer_case_insensitive() {
    let query = Parser::parse("train quantizer on my_collection with (m=8)").unwrap();
    assert!(query.is_train());
}

#[test]
fn test_train_is_not_select() {
    let query = Parser::parse("TRAIN QUANTIZER ON my_collection WITH (m=8)").unwrap();
    assert!(!query.is_select_query());
    assert!(!query.is_dml_query());
    assert!(!query.is_match_query());
}
