//! Tests for [`deserialize_id`](super::deserialize_id).
//!
//! Exercised through [`Link`], the simplest type whose `target` field uses
//! `#[serde(deserialize_with = "deserialize_id")]` — `mcp/dto.rs`'s
//! `relate`/`forget`/`feedback` params reuse the exact same function, so a
//! fix proven here covers those call sites too.

use super::Link;
use serde_json::json;

#[test]
fn deserialize_id_leading_whitespace_string_parses() {
    // Some MCP harnesses (observed with Claude Code, 2026-07-20) coerce any
    // all-digit scalar to a JSON number, losing precision above 2^53. A
    // client working around this by prefixing whitespace keeps the value a
    // JSON string, but the id must still parse.
    let link: Link = serde_json::from_value(json!({
        "target": " 123",
        "relation": "r",
    }))
    .expect("leading whitespace around a decimal id must be tolerated");

    assert_eq!(link.target, 123);
}

#[test]
fn deserialize_id_trailing_whitespace_string_parses() {
    let link: Link = serde_json::from_value(json!({
        "target": "123 ",
        "relation": "r",
    }))
    .expect("trailing whitespace around a decimal id must be tolerated");

    assert_eq!(link.target, 123);
}

#[test]
fn deserialize_id_whitespace_padded_max_u64_parses() {
    let link: Link = serde_json::from_value(json!({
        "target": " 18446744073709551615 ",
        "relation": "r",
    }))
    .expect("whitespace-padded u64::MAX must be tolerated");

    assert_eq!(link.target, u64::MAX);
}

#[test]
fn deserialize_id_plus_prefixed_string_parses() {
    // Non-regression: the documented workaround for the whitespace problem
    // above. A '+'-prefixed decimal string is not a JSON number, so a
    // digit-coercing harness leaves it as a string — and `u64::from_str`
    // already accepts the leading '+', so this must keep working
    // independently of the trim fix.
    let link: Link = serde_json::from_value(json!({
        "target": "+123",
        "relation": "r",
    }))
    .expect("a '+'-prefixed decimal id must keep working");

    assert_eq!(link.target, 123);
}
