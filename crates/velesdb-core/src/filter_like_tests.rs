//! TDD tests for LIKE/ILIKE filter operators (EPIC-CORE-002).
//!
//! These tests define the expected behavior for SQL-like pattern matching
//! with % (zero or more chars) and _ (single char) wildcards.

use crate::filter::{Condition, Filter};
use serde_json::json;

// =============================================================================
// AC1: LIKE operator (case-sensitive)
// =============================================================================

#[test]
fn test_like_exact_match() {
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "Paris");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_percent_wildcard_prefix() {
    // %aris matches "Paris", "Baris", etc.
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "%aris");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_percent_wildcard_suffix() {
    // Par% matches "Paris", "Parrot", etc.
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "Par%");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_percent_wildcard_both() {
    // %ari% matches "Paris", "Caribou", etc.
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "%ari%");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_underscore_wildcard() {
    // P_ris matches "Paris", "Peris", but not "Pris"
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "P_ris");
    assert!(condition.matches(&payload));

    let payload_short = json!({"name": "Pris"});
    assert!(!condition.matches(&payload_short));
}

#[test]
fn test_like_mixed_wildcards() {
    // P%_s matches "Paris", "Prints", etc.
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "P%_s");
    assert!(condition.matches(&payload));

    let payload2 = json!({"name": "Prints"});
    assert!(condition.matches(&payload2));
}

#[test]
fn test_like_case_sensitive() {
    // LIKE is case-sensitive
    let payload = json!({"name": "Paris"});
    let condition = Condition::like("name", "paris");
    assert!(!condition.matches(&payload));
}

#[test]
fn test_like_no_match() {
    let payload = json!({"name": "London"});
    let condition = Condition::like("name", "Par%");
    assert!(!condition.matches(&payload));
}

#[test]
fn test_like_empty_pattern() {
    let payload = json!({"name": ""});
    let condition = Condition::like("name", "");
    assert!(condition.matches(&payload));

    let payload_nonempty = json!({"name": "Paris"});
    assert!(!condition.matches(&payload_nonempty));
}

#[test]
fn test_like_only_percent() {
    // % matches anything
    let payload = json!({"name": "Anything"});
    let condition = Condition::like("name", "%");
    assert!(condition.matches(&payload));

    let empty = json!({"name": ""});
    assert!(condition.matches(&empty));
}

// =============================================================================
// AC2: ILIKE operator (case-insensitive)
// =============================================================================

#[test]
fn test_ilike_case_insensitive() {
    let payload = json!({"name": "Paris"});

    // Should match regardless of case
    assert!(Condition::ilike("name", "paris").matches(&payload));
    assert!(Condition::ilike("name", "PARIS").matches(&payload));
    assert!(Condition::ilike("name", "PaRiS").matches(&payload));
}

#[test]
fn test_ilike_with_wildcards() {
    let payload = json!({"name": "Paris"});

    assert!(Condition::ilike("name", "%ARIS").matches(&payload));
    assert!(Condition::ilike("name", "par%").matches(&payload));
    assert!(Condition::ilike("name", "p_ris").matches(&payload));
}

#[test]
fn test_ilike_no_match() {
    let payload = json!({"name": "London"});
    assert!(!Condition::ilike("name", "%aris").matches(&payload));
}

// =============================================================================
// AC3: Edge cases
// =============================================================================

#[test]
fn test_like_null_field() {
    let payload = json!({"name": null});
    let condition = Condition::like("name", "%");
    assert!(!condition.matches(&payload));
}

#[test]
fn test_like_missing_field() {
    let payload = json!({"other": "value"});
    let condition = Condition::like("name", "%");
    assert!(!condition.matches(&payload));
}

#[test]
fn test_like_non_string_field() {
    let payload = json!({"count": 42});
    let condition = Condition::like("count", "%");
    assert!(!condition.matches(&payload));
}

#[test]
fn test_like_nested_field() {
    let payload = json!({
        "address": {
            "city": "Paris"
        }
    });
    let condition = Condition::like("address.city", "Par%");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_special_regex_chars() {
    // Ensure regex special chars in pattern are treated literally
    let payload = json!({"expr": "a+b*c?"});
    let condition = Condition::like("expr", "a+b*c?");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_escaped_percent() {
    // \% should match literal %
    let payload = json!({"discount": "50%"});
    let condition = Condition::like("discount", "50\\%");
    assert!(condition.matches(&payload));
}

#[test]
fn test_like_escaped_underscore() {
    // \_ should match literal _
    let payload = json!({"code": "A_B"});
    let condition = Condition::like("code", "A\\_B");
    assert!(condition.matches(&payload));
}

// =============================================================================
// AC4: Filter integration
// =============================================================================

#[test]
fn test_filter_with_like() {
    let filter = Filter::new(Condition::like("name", "%aris"));
    let payload = json!({"name": "Paris", "country": "France"});
    assert!(filter.matches(&payload));
}

#[test]
fn test_filter_like_combined_with_other_conditions() {
    let filter = Filter::new(Condition::and(vec![
        Condition::like("name", "%aris"),
        Condition::eq("country", "France"),
    ]));

    let paris = json!({"name": "Paris", "country": "France"});
    assert!(filter.matches(&paris));

    let wrong_country = json!({"name": "Paris", "country": "Spain"});
    assert!(!filter.matches(&wrong_country));
}

// =============================================================================
// AC5: Serialization/Deserialization
// =============================================================================

#[test]
fn test_like_serialization() {
    let condition = Condition::like("name", "Par%");
    let json = serde_json::to_string(&condition).unwrap();
    assert!(json.contains("\"type\":\"like\""));
    assert!(json.contains("\"pattern\":\"Par%\""));
}

#[test]
fn test_ilike_serialization() {
    let condition = Condition::ilike("name", "par%");
    let json = serde_json::to_string(&condition).unwrap();
    assert!(json.contains("\"type\":\"ilike\""));
}

#[test]
fn test_like_deserialization() {
    let json = r#"{"type":"like","field":"name","pattern":"Par%"}"#;
    let condition: Condition = serde_json::from_str(json).unwrap();
    let payload = json!({"name": "Paris"});
    assert!(condition.matches(&payload));
}

#[test]
fn test_ilike_deserialization() {
    let json = r#"{"type":"ilike","field":"name","pattern":"par%"}"#;
    let condition: Condition = serde_json::from_str(json).unwrap();
    let payload = json!({"name": "Paris"});
    assert!(condition.matches(&payload));
}
