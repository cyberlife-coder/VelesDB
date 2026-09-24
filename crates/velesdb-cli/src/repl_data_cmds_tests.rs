//! `.bench` searches at the session's quality, or at the configured default
//! when the session set none, and reports a failed query (#2303).

use tempfile::TempDir;

use super::cmd_bench;
use crate::repl::ReplConfig;
use crate::repl_commands::CommandResult;
use crate::repl_execute::repl_execute_tests::seed_docs_refusing_perfect;

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

#[test]
fn bench_at_an_untouched_session_runs_every_query() {
    assert!(matches!(bench_with(&[]), CommandResult::Continue));
}
