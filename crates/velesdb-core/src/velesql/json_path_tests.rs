//! Tests for `json_path` module - JSON path parsing and extraction.

use super::json_path::*;
use serde_json::json;

#[test]
fn test_parse_simple() {
    let path = JsonPath::parse("name").unwrap();
    assert_eq!(path.segments.len(), 1);
    assert_eq!(path.segments[0], PathSegment::Property("name".to_string()));
    assert!(path.is_simple());
}

#[test]
fn test_parse_nested() {
    let path = JsonPath::parse("metadata.source").unwrap();
    assert_eq!(path.segments.len(), 2);
    assert_eq!(
        path.segments[0],
        PathSegment::Property("metadata".to_string())
    );
    assert_eq!(
        path.segments[1],
        PathSegment::Property("source".to_string())
    );
    assert!(!path.is_simple());
}

#[test]
fn test_parse_deep_nested() {
    let path = JsonPath::parse("a.b.c.d.e").unwrap();
    assert_eq!(path.segments.len(), 5);
}

#[test]
fn test_parse_array_index() {
    let path = JsonPath::parse("items[0]").unwrap();
    assert_eq!(path.segments.len(), 2);
    assert_eq!(path.segments[0], PathSegment::Property("items".to_string()));
    assert_eq!(path.segments[1], PathSegment::Index(0));
}

#[test]
fn test_parse_array_with_property() {
    let path = JsonPath::parse("items[0].sku").unwrap();
    assert_eq!(path.segments.len(), 3);
    assert_eq!(path.segments[0], PathSegment::Property("items".to_string()));
    assert_eq!(path.segments[1], PathSegment::Index(0));
    assert_eq!(path.segments[2], PathSegment::Property("sku".to_string()));
}

#[test]
fn test_parse_empty_error() {
    assert!(matches!(JsonPath::parse(""), Err(JsonPathError::EmptyPath)));
    assert!(matches!(
        JsonPath::parse("   "),
        Err(JsonPathError::EmptyPath)
    ));
}

#[test]
fn test_parse_double_dot_error() {
    assert!(matches!(
        JsonPath::parse("a..b"),
        Err(JsonPathError::EmptySegment)
    ));
}

#[test]
fn test_parse_unclosed_bracket_error() {
    assert!(matches!(
        JsonPath::parse("items[0"),
        Err(JsonPathError::UnclosedBracket)
    ));
}

#[test]
fn test_parse_invalid_index_error() {
    assert!(matches!(
        JsonPath::parse("items[abc]"),
        Err(JsonPathError::InvalidArrayIndex(_))
    ));
}

#[test]
fn test_extract_simple() {
    let doc = json!({"name": "Alice", "age": 30});
    let path = JsonPath::parse("name").unwrap();
    assert_eq!(path.extract(&doc), Some(&json!("Alice")));
}

#[test]
fn test_extract_nested() {
    let doc = json!({
        "metadata": {
            "source": "web",
            "campaign": "summer"
        }
    });
    let path = JsonPath::parse("metadata.source").unwrap();
    assert_eq!(path.extract(&doc), Some(&json!("web")));
}

#[test]
fn test_extract_deep_nested() {
    let doc = json!({
        "profile": {
            "address": {
                "city": "Paris",
                "country": "FR"
            }
        }
    });
    let path = JsonPath::parse("profile.address.city").unwrap();
    assert_eq!(path.extract(&doc), Some(&json!("Paris")));
}

#[test]
fn test_extract_array() {
    let doc = json!({
        "items": [
            {"sku": "A1", "qty": 2},
            {"sku": "B2", "qty": 1}
        ]
    });
    let path = JsonPath::parse("items[0].sku").unwrap();
    assert_eq!(path.extract(&doc), Some(&json!("A1")));

    let path = JsonPath::parse("items[1].sku").unwrap();
    assert_eq!(path.extract(&doc), Some(&json!("B2")));
}

#[test]
fn test_extract_missing_returns_none() {
    let doc = json!({"name": "Alice"});
    let path = JsonPath::parse("nonexistent").unwrap();
    assert_eq!(path.extract(&doc), None);

    let path = JsonPath::parse("name.nested").unwrap();
    assert_eq!(path.extract(&doc), None);
}

#[test]
fn test_extract_or_null() {
    let doc = json!({"name": "Alice"});
    let path = JsonPath::parse("nonexistent").unwrap();
    assert_eq!(path.extract_or_null(&doc), serde_json::Value::Null);

    let path = JsonPath::parse("name").unwrap();
    assert_eq!(path.extract_or_null(&doc), json!("Alice"));
}

#[test]
fn test_root_property() {
    let path = JsonPath::parse("metadata.source").unwrap();
    assert_eq!(path.root_property(), Some("metadata"));

    let path = JsonPath::parse("[0].field").unwrap();
    assert_eq!(path.root_property(), None);
}

#[test]
fn test_tail() {
    let path = JsonPath::parse("a.b.c").unwrap();
    let tail = path.tail();
    assert_eq!(tail.segments.len(), 2);
    assert_eq!(tail.to_string(), "b.c");
}

#[test]
fn test_display() {
    let path = JsonPath::parse("metadata.source").unwrap();
    assert_eq!(path.to_string(), "metadata.source");

    let path = JsonPath::parse("items[0].sku").unwrap();
    assert_eq!(path.to_string(), "items[0].sku");
}

#[test]
fn test_serialization() {
    let path = JsonPath::parse("metadata.source").unwrap();
    let json = serde_json::to_string(&path).unwrap();
    let parsed: JsonPath = serde_json::from_str(&json).unwrap();
    assert_eq!(path, parsed);
}
