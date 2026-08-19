use super::*;
use serde_json::json;

#[test]
fn test_payload_contains_text_specific_field() {
    let payload = json!({"title": "Hello World", "content": "Some text"});
    assert!(payload_contains_text(&payload, "hello", Some("title")));
    assert!(!payload_contains_text(&payload, "hello", Some("content")));
}

#[test]
fn test_payload_contains_text_all_fields() {
    let payload = json!({"title": "Hello", "content": "World"});
    assert!(payload_contains_text(&payload, "hello", None));
    assert!(payload_contains_text(&payload, "world", None));
}

#[test]
fn test_search_all_fields_nested() {
    let payload = json!({
        "metadata": {
            "author": "John Doe",
            "tags": ["rust", "wasm"]
        }
    });
    assert!(search_all_fields(&payload, "john"));
    assert!(search_all_fields(&payload, "rust"));
}

#[test]
fn test_value_contains_text_array() {
    let value = json!(["apple", "banana", "cherry"]);
    assert!(value_contains_text(&value, "banana"));
    assert!(!value_contains_text(&value, "orange"));
}

#[test]
fn test_case_insensitive() {
    let payload = json!({"name": "VelesDB"});
    assert!(payload_contains_text(&payload, "velesdb", None));
    assert!(payload_contains_text(
        &payload,
        "VELESDB".to_lowercase().as_str(),
        None
    ));
}
