//! Unit tests for the global output options module (`output.rs`).

use super::*;
use serde_json::json;

#[test]
fn test_resolve_format_global_wins() {
    assert_eq!(resolve_format(Some(GlobalFormat::Jsonl), "table"), "jsonl");
    assert_eq!(resolve_format(Some(GlobalFormat::Csv), "json"), "csv");
}

#[test]
fn test_resolve_format_fallback_when_unset() {
    assert_eq!(resolve_format(None, "table"), "table");
    assert_eq!(resolve_format(None, "tree"), "tree");
}

#[test]
fn test_is_machine() {
    assert!(is_machine("json"));
    assert!(is_machine("JSON"));
    assert!(is_machine("jsonl"));
    assert!(is_machine("Csv"));
    assert!(!is_machine("table"));
    assert!(!is_machine("tree"));
    assert!(!is_machine("toml"));
}

#[test]
fn test_machine_lines_json_pretty() {
    let lines = machine_lines(&json!({"a": 1}), "json").unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains('\n'), "pretty JSON is multi-line");
    assert!(lines[0].contains("\"a\": 1"));
}

#[test]
fn test_machine_lines_jsonl_array() {
    let lines = machine_lines(&json!([{"a": 1}, {"a": 2}]), "jsonl").unwrap();
    assert_eq!(
        lines,
        vec!["{\"a\":1}".to_string(), "{\"a\":2}".to_string()]
    );
}

#[test]
fn test_machine_lines_jsonl_scalar() {
    let lines = machine_lines(&json!(42), "jsonl").unwrap();
    assert_eq!(lines, vec!["42".to_string()]);
}

#[test]
fn test_machine_lines_csv_objects() {
    let lines = machine_lines(&json!([{"a": 1, "b": "x"}, {"a": 2}]), "csv").unwrap();
    assert_eq!(lines[0], "a,b");
    assert_eq!(lines[1], "1,\"x\"");
    assert_eq!(lines[2], "2,null");
}

#[test]
fn test_machine_lines_csv_scalar_wraps() {
    let lines = machine_lines(&json!(7), "csv").unwrap();
    assert_eq!(lines, vec!["7".to_string()]);
}

#[test]
fn test_machine_lines_rejects_table() {
    assert!(machine_lines(&json!([]), "table").is_err());
}

#[test]
fn test_exit_code_not_found_is_3() {
    let err = anyhow::Error::new(velesdb_core::Error::CollectionNotFound("c".into()));
    assert_eq!(exit_code(&err), 3);
    let err = anyhow::Error::new(velesdb_core::Error::PointNotFound(7));
    assert_eq!(exit_code(&err), 3);
    let err = anyhow::Error::new(velesdb_core::Error::NodeNotFound(1));
    assert_eq!(exit_code(&err), 3);
    let err = anyhow::Error::new(velesdb_core::Error::EdgeNotFound(2));
    assert_eq!(exit_code(&err), 3);
}

#[test]
fn test_exit_code_io_is_4() {
    let io = std::io::Error::new(std::io::ErrorKind::NotFound, "file");
    let err = anyhow::Error::new(velesdb_core::Error::Io(io));
    assert_eq!(exit_code(&err), 4);
    let err = anyhow::Error::new(std::io::Error::other("x"));
    assert_eq!(exit_code(&err), 4);
}

#[test]
fn test_exit_code_parse_is_5() {
    let err = anyhow::Error::new(velesdb_core::Error::Query("syntax error".into()));
    assert_eq!(exit_code(&err), 5);
}

#[test]
fn test_exit_code_generic_is_1() {
    let err = anyhow::Error::new(velesdb_core::Error::Internal("boom".into()));
    assert_eq!(exit_code(&err), 1);
    let err = anyhow::anyhow!("plain failure");
    assert_eq!(exit_code(&err), 1);
}

#[test]
fn test_init_records_quiet() {
    init(OutputOptions {
        format: Some(GlobalFormat::Json),
        quiet: true,
        no_color: true,
    });
    assert!(quiet());
    // OnceLock: a second init is a no-op, options stay as first set.
    init(OutputOptions::default());
    assert!(quiet());
    assert_eq!(effective_format("table"), "json");
}
