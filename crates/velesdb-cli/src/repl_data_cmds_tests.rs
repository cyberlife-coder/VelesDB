//! `.bench` searches at the session's quality, or at the configured default
//! when the session set none, and reports a failed query (#2303).

use tempfile::TempDir;

use velesdb_core::SearchQuality;

use super::{bench_quality, cmd_bench};
use crate::repl::ReplConfig;
use crate::repl_commands::CommandResult;
use crate::session::SessionSettings;
use crate::test_fixtures::{seed_docs_configured, seed_docs_refusing_perfect};

/// Runs `.bench` over three queries on a `docs` collection whose Perfect
/// (brute-force) mode refuses it: a search that really runs at `perfect`
/// fails, one that runs at any other quality does not.
fn bench_with(settings: &[(&str, &str)]) -> CommandResult {
    let dir = TempDir::new().expect("test: temp dir");
    let db = seed_docs_refusing_perfect(&dir, 3);
    let mut config = ReplConfig::default();
    for (key, value) in settings {
        config.session.set(key, value).expect("test: \\set");
    }
    cmd_bench(&db, &config, &[".bench", "docs", "3", "1"])
}

#[test]
fn bench_runs_at_the_session_mode_and_reports_the_refusal() {
    match bench_with(&[("mode", "perfect")]) {
        CommandResult::Error(message) => assert!(message.contains("Perfect"), "{message}"),
        _ => panic!("`.bench` at mode perfect ran past the Perfect-mode cap"),
    }
}

#[test]
fn bench_runs_at_the_session_ef_search_over_its_mode() {
    // `ef_search` wins over the mode, as in a query: no Perfect search runs.
    let result = bench_with(&[("mode", "perfect"), ("ef_search", "64")]);
    assert!(
        matches!(result, CommandResult::Continue),
        "the session ef_search was ignored"
    );
}

/// An untouched session prints the default the database was opened with
/// and takes the `col.search` arm, which applies it; a session that set a
/// quality prints it and takes the `search_with_quality` arm. The search arm
/// itself is left unpinned on purpose: it is the call an untouched query
/// makes, and no test-sized collection tells it from a `balanced` search.
#[test]
fn bench_at_an_untouched_session_prints_the_configured_default() {
    let dir = TempDir::new().expect("test: temp dir");
    let db = seed_docs_configured(&dir, "[search]\ndefault_mode = \"fast\"\n", 3);
    let untouched = SessionSettings::new();
    assert_eq!(
        bench_quality(&db, &untouched),
        (None, "fast (configured default)".to_string())
    );
    let result = cmd_bench(&db, &ReplConfig::default(), &[".bench", "docs", "3", "1"]);
    assert!(matches!(result, CommandResult::Continue));

    let mut set = SessionSettings::new();
    set.set("ef_search", "64").expect("test: \\set ef_search");
    assert_eq!(
        bench_quality(&db, &set),
        (Some(SearchQuality::Custom(64)), "custom:64".to_string())
    );
}

#[test]
fn export_writes_every_point_of_a_vector_collection_payload_or_not() {
    let dir = TempDir::new().expect("test: temp dir");
    let db = velesdb_core::Database::open(dir.path()).expect("test: open");
    db.create_vector_collection("docs", 2, velesdb_core::DistanceMetric::Cosine)
        .expect("test: create");
    db.get_vector_collection("docs")
        .expect("test: docs")
        .upsert(vec![
            velesdb_core::Point::new(1, vec![1.0, 0.0], Some(serde_json::json!({"k": "a"}))),
            velesdb_core::Point::new(2, vec![0.0, 1.0], None),
        ])
        .expect("test: seed");
    let file = dir.path().join("docs.json");
    let file = file.to_str().expect("test: utf-8 path");

    let result = super::cmd_export(&db, &[".export", "docs", file]);

    assert!(matches!(result, CommandResult::Continue));
    let records: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(file).expect("test: read export"))
            .expect("test: parse export");
    let mut ids: Vec<u64> = records
        .iter()
        .filter_map(|r| r.get("id").and_then(serde_json::Value::as_u64))
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "the point without payload was left out");
}
