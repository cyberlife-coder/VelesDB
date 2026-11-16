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
    assert_eq!(path.segments[0], PathSegment::Property("a".to_string()));
    assert_eq!(path.segments[1], PathSegment::Property("b".to_string()));
    assert_eq!(path.segments[2], PathSegment::Property("c".to_string()));
    assert_eq!(path.segments[3], PathSegment::Property("d".to_string()));
    assert_eq!(path.segments[4], PathSegment::Property("e".to_string()));
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

// ── payload_path / column_value: the dotted path every clause reads ──

#[test]
fn payload_path_walks_nested_objects() {
    let payload = json!({"meta": {"lang": "fr", "tags": ["a"]}, "n": 1});
    assert_eq!(payload_path(&payload, "n"), Some(&json!(1)));
    assert_eq!(payload_path(&payload, "meta.lang"), Some(&json!("fr")));
    assert_eq!(payload_path(&payload, "meta.missing"), None);
}

/// A segment names an object key: it never indexes an array nor reads
/// through a scalar.
#[test]
fn payload_path_reads_only_through_objects() {
    let payload = json!({"tags": ["a", "b"], "n": 1});
    assert_eq!(payload_path(&payload, "tags.0"), None);
    assert_eq!(payload_path(&payload, "n.x"), None);
    assert_eq!(payload_path(&json!(["a"]), "0"), None);
}

/// No depth ceiling: a path is as long as the query wrote it.
#[test]
fn payload_path_follows_a_path_of_any_depth() {
    let mut payload = json!("leaf");
    for _ in 0..12 {
        payload = json!({ "k": payload });
    }
    let path = ["k"; 12].join(".");
    assert_eq!(payload_path(&payload, &path), Some(&json!("leaf")));
}

#[test]
fn column_value_reads_the_point_id_before_the_payload() {
    let row = crate::point::SearchResult::new(
        crate::point::Point::metadata_only(7, json!({"id": "shadow", "meta": {"x": 2}})),
        0.5,
    );
    assert_eq!(column_value(&row, "id"), json!(7));
    assert_eq!(column_value(&row, "meta.x"), json!(2));
    assert_eq!(column_value(&row, "absent"), serde_json::Value::Null);
}
