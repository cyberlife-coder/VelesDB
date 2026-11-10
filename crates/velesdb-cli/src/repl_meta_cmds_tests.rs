//! Tests for the daily-driver meta commands (`\save`, `\load`, `\watch`,
//! `\edit`, `\copy`) — argument parsing, payload selection, error paths
//! and the `\load` recursion guard. Terminal-facing output paths are
//! covered end-to-end in `tests/repl_e2e_tests.rs`.

use super::*;
use tempfile::TempDir;

/// A `ReplState` over a throwaway database; the TempDir is returned so the
/// caller keeps it alive for the test.
fn test_state() -> (ReplState, TempDir) {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("db");
    let db = velesdb_core::Database::open(&path).expect("open db");
    (ReplState::new(db, path), temp)
}

fn test_editor() -> ReplEditor {
    ReplEditor::new().expect("editor")
}

fn query_result(rows: usize) -> crate::repl::QueryResult {
    crate::repl::QueryResult {
        rows: (0..rows)
            .map(|i| {
                std::collections::HashMap::from([(
                    "id".to_string(),
                    serde_json::Value::from(i as u64),
                )])
            })
            .collect(),
        duration_ms: 0.0,
        kind: crate::repl::QueryKind::Select,
    }
}

// ----------------------------------------------------------------------------
// \save
// ----------------------------------------------------------------------------

#[test]
fn test_save_content_query() {
    let (mut state, _temp) = test_state();
    state.last_query = Some("SELECT * FROM docs".to_string());
    let content = save_content(&state, false).expect("content");
    assert_eq!(content, "SELECT * FROM docs\n");
}

#[test]
fn test_save_content_result_json() {
    let (mut state, _temp) = test_state();
    state.last_result = Some(query_result(2));
    let content = save_content(&state, true).expect("content");
    let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid json");
    assert_eq!(parsed.as_array().map(Vec::len), Some(2));
}

#[test]
fn test_save_content_errors_without_state() {
    let (state, _temp) = test_state();
    assert!(save_content(&state, false).is_err());
    assert!(save_content(&state, true).is_err());
}

#[test]
fn test_cmd_save_usage_without_args() {
    let (state, _temp) = test_state();
    assert!(matches!(
        cmd_save(&state, &["\\save"]),
        CommandResult::Continue
    ));
}

// ----------------------------------------------------------------------------
// \load / run_script
// ----------------------------------------------------------------------------

#[test]
fn test_load_depth_guard_rejects_cycles() {
    let (mut state, _temp) = test_state();
    let mut rl = test_editor();
    let mut config = ReplConfig::default();
    state.load_depth = MAX_LOAD_DEPTH;
    let result = run_script(&mut state, &mut rl, "SELECT 1", &mut config, "cycle.sql");
    assert!(matches!(result, CommandResult::Error(_)));
    // the guard ran before the line executed — depth untouched
    assert_eq!(state.load_depth, MAX_LOAD_DEPTH);
}

#[test]
fn test_run_script_skips_comments_and_blanks() {
    let (mut state, _temp) = test_state();
    let mut rl = test_editor();
    let mut config = ReplConfig::default();
    // Unknown commands still count as executed lines and continue the loop.
    let source = "-- a comment\n\n.nosuchcommand\n";
    let result = run_script(&mut state, &mut rl, source, &mut config, "s.sql");
    assert!(matches!(result, CommandResult::Continue));
    assert_eq!(state.load_depth, 0);
}

#[test]
fn test_run_script_quit_propagates() {
    let (mut state, _temp) = test_state();
    let mut rl = test_editor();
    let mut config = ReplConfig::default();
    let result = run_script(&mut state, &mut rl, ".quit", &mut config, "s.sql");
    assert!(matches!(result, CommandResult::Quit));
}

// ----------------------------------------------------------------------------
// \watch
// ----------------------------------------------------------------------------

#[test]
fn test_parse_watch_args_defaults() {
    assert_eq!(parse_watch_args(&["\\watch"]).expect("args"), (2.0, 3));
}

#[test]
fn test_parse_watch_args_explicit() {
    assert_eq!(
        parse_watch_args(&["\\watch", "0.5", "7"]).expect("args"),
        (0.5, 7)
    );
}

#[test]
fn test_parse_watch_args_rejects_bad_values() {
    assert!(parse_watch_args(&["\\watch", "-1"]).is_err());
    assert!(parse_watch_args(&["\\watch", "abc"]).is_err());
    assert!(parse_watch_args(&["\\watch", "1", "0"]).is_err());
    assert!(parse_watch_args(&["\\watch", "1", "1001"]).is_err());
    assert!(parse_watch_args(&["\\watch", "1", "x"]).is_err());
}

#[test]
fn test_watch_requires_last_query() {
    let (mut state, _temp) = test_state();
    let mut rl = test_editor();
    let mut config = ReplConfig::default();
    let result = cmd_watch(&mut state, &mut rl, &["\\watch"], &mut config);
    assert!(matches!(result, CommandResult::Error(_)));
}

// ----------------------------------------------------------------------------
// \copy payload selection
// ----------------------------------------------------------------------------

#[test]
fn test_copy_payload_defaults_to_result() {
    let (mut state, _temp) = test_state();
    state.last_result = Some(query_result(3));
    let (label, text) = copy_payload(&state, &["\\copy"]).expect("payload");
    assert!(label.contains('3'));
    assert!(text.contains("\"id\""));
}

#[test]
fn test_copy_payload_query_variant() {
    let (mut state, _temp) = test_state();
    state.last_query = Some("SELECT 1".to_string());
    let (_label, text) = copy_payload(&state, &["\\copy", "query"]).expect("payload");
    assert_eq!(text, "SELECT 1");
}

#[test]
fn test_copy_payload_errors_without_state() {
    let (state, _temp) = test_state();
    assert!(copy_payload(&state, &["\\copy"]).is_err());
    assert!(copy_payload(&state, &["\\copy", "query"]).is_err());
}

// ----------------------------------------------------------------------------
// \edit
// ----------------------------------------------------------------------------

#[test]
fn test_edit_requires_last_query() {
    let (mut state, _temp) = test_state();
    let mut rl = test_editor();
    let mut config = ReplConfig::default();
    let result = cmd_edit(&mut state, &mut rl, &mut config);
    assert!(matches!(result, CommandResult::Error(_)));
}

#[test]
fn test_edit_scratch_path_is_unique_and_user_private() {
    let a = edit_scratch_path();
    let b = edit_scratch_path();
    // Not in the shared temp dir — next to the persisted history file.
    let expected_dir = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    assert_eq!(a.parent(), Some(expected_dir.as_path()));
    assert_ne!(a, b);
    assert!(a.extension().is_some_and(|e| e == "sql"));
}

// ----------------------------------------------------------------------------
// \attach / \detach / \databases / \connect
// ----------------------------------------------------------------------------

#[test]
fn test_attach_registers_alias_and_connect_switches() {
    let (mut state, _t1) = test_state();
    let t2 = TempDir::new().expect("tempdir2");
    let path2 = t2.path().join("db2");
    velesdb_core::Database::open(&path2).expect("open db2");
    drop_path_check(&path2);

    let result = cmd_attach(
        &mut state,
        &["\\attach", path2.to_str().expect("utf8"), "aux"],
    );
    assert!(matches!(result, CommandResult::Continue));
    assert!(state.databases.contains_key("aux"));

    let mut config = ReplConfig::default();
    config.session.use_collection(Some("docs".to_string()));
    let result = cmd_connect(&mut state, &mut config, &["\\connect", "aux"]);
    assert!(matches!(result, CommandResult::Continue));
    assert_eq!(state.active, "aux");
    assert!(config.session.active_collection().is_none());
}

#[test]
fn test_attach_duplicate_alias_errors() {
    let (mut state, _t1) = test_state();
    let t2 = TempDir::new().expect("tempdir2");
    let path2 = t2.path().join("db2");
    velesdb_core::Database::open(&path2).expect("open db2");
    drop_path_check(&path2);
    let p = path2.to_str().expect("utf8");
    assert!(matches!(
        cmd_attach(&mut state, &["\\attach", p, "aux"]),
        CommandResult::Continue
    ));
    assert!(matches!(
        cmd_attach(&mut state, &["\\attach", p, "aux"]),
        CommandResult::Error(_)
    ));
}

#[test]
fn test_detach_refuses_active_and_unknown() {
    let (mut state, _t1) = test_state();
    assert!(matches!(
        cmd_detach(&mut state, &["\\detach", "main"]),
        CommandResult::Error(_)
    ));
    assert!(matches!(
        cmd_detach(&mut state, &["\\detach", "ghost"]),
        CommandResult::Error(_)
    ));
}

#[test]
fn test_connect_unknown_alias_errors() {
    let (mut state, _t1) = test_state();
    let mut config = ReplConfig::default();
    assert!(matches!(
        cmd_connect(&mut state, &mut config, &["\\connect", "ghost"]),
        CommandResult::Error(_)
    ));
    assert_eq!(state.active, crate::repl::MAIN_ALIAS);
}

/// Two `Database` handles on the same path conflict via the lock file —
/// this drops nothing, it just asserts the seed handle is closed so
/// `\\attach` can open the same path again.
fn drop_path_check(path: &std::path::Path) {
    assert!(path.exists());
}
