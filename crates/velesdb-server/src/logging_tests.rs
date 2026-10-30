use super::*;
use tracing_subscriber::Registry;

// ---------------------------------------------------------------------------
// Precedence: RUST_LOG > VELESDB_LOGGING_LEVEL / [logging] level > default
// ---------------------------------------------------------------------------

#[test]
fn test_rust_log_wins_over_a_configured_level() {
    assert_eq!(
        filter_directives(Some("velesdb_core=trace"), Some(LogLevel::Warn)),
        "velesdb_core=trace"
    );
}

#[test]
fn test_configured_level_applies_without_rust_log() {
    assert_eq!(filter_directives(None, Some(LogLevel::Warn)), "warn");
}

#[test]
fn test_nothing_configured_keeps_the_historical_filter() {
    assert_eq!(filter_directives(None, None), "info,tower_http=debug");
}

#[test]
fn test_level_names_round_trip() {
    for name in LogLevel::NAMES {
        let level = LogLevel::parse(name, "test").expect("test: valid level");
        assert_eq!(level.as_str(), name);
    }
}

// ---------------------------------------------------------------------------
// Format and file: what lands in the file
// ---------------------------------------------------------------------------

/// Emits one event through a subscriber built from `fmt_layer(format, ...)`
/// that appends to `path`, and returns the file's contents.
fn emit_to(path: &std::path::Path, format: LogFormat) -> String {
    let writer = append_writer(path).expect("test: open log file");
    let subscriber = Registry::default().with(fmt_layer::<Registry>(format, writer, false));
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(answer = 42, "hello from the test");
    });
    std::fs::read_to_string(path).expect("test: read log file")
}

#[test]
fn test_json_format_writes_one_json_object_per_event() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let contents = emit_to(&dir.path().join("json.log"), LogFormat::Json);
    let line = contents.lines().next().expect("test: one line");
    let event: serde_json::Value = serde_json::from_str(line).expect("test: JSON line");
    assert_eq!(event["level"], "INFO");
    assert_eq!(event["fields"]["message"], "hello from the test");
    assert_eq!(event["fields"]["answer"], 42);
}

#[test]
fn test_text_format_into_a_file_is_plain_text_without_colours() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let contents = emit_to(&dir.path().join("text.log"), LogFormat::Text);
    assert!(
        contents.contains("hello from the test answer=42"),
        "{contents}"
    );
    assert!(serde_json::from_str::<serde_json::Value>(contents.trim()).is_err());
    assert!(!contents.contains('\u{1b}'), "{contents:?}");
}

#[test]
fn test_log_file_is_appended_to_not_truncated() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let path = dir.path().join("append.log");
    std::fs::write(&path, "previous run\n").expect("test: seed");
    let contents = emit_to(&path, LogFormat::Json);
    assert!(contents.starts_with("previous run\n"), "{contents}");
    assert_eq!(contents.lines().count(), 2);
}

#[test]
fn test_unopenable_log_file_is_an_error_naming_the_setting() {
    let dir = tempfile::tempdir().expect("test: tempdir");
    let err = append_writer(&dir.path().join("missing-dir").join("x.log"))
        .expect_err("test: parent directory does not exist");
    assert!(err.to_string().contains("logging.file"), "{err}");
}
