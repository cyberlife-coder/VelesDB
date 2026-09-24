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
