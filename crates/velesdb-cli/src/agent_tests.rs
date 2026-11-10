//! Agent tests — offline mode is always exercised (no LLM ever required in
//! tests). Single-threaded per repo convention.

use super::*;
use tempfile::TempDir;

fn demo_db() -> (TempDir, Database) {
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    db.create_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("create docs");
    (dir, db)
}

#[test]
fn offline_run_lists_collections_and_composes_a_sourced_answer() {
    std::env::set_var("VELESDB_AGENT_URL", "http://127.0.0.1:1");
    let (_dir, db) = demo_db();
    let report = run(&db, "describe docs");
    assert_eq!(report.backend, "offline");
    assert!(report.iterations.len() >= 2);
    assert_eq!(report.iterations[0].action, "list()");
    assert!(!report.sources.is_empty());
    assert!(!report.answer.is_empty());
}

#[test]
fn iterations_never_exceed_the_bound() {
    std::env::set_var("VELESDB_AGENT_URL", "http://127.0.0.1:1");
    let (_dir, db) = demo_db();
    let report = run(&db, "tell me everything");
    assert!(report.iterations.len() < MAX_ITERATIONS);
}

#[test]
fn unknown_tool_is_refused_by_the_whitelist() {
    let (_dir, db) = demo_db();
    let obs = tools::call(&db, "rm_rf", "");
    assert!(obs.is_err());
    assert!(obs.unwrap_err().contains("whitelist"));
}

#[test]
fn every_whitelisted_tool_dispatches() {
    let (_dir, db) = demo_db();
    for name in tools::TOOL_NAMES {
        // All tools except `list` need args; assert only that dispatch
        // reaches the tool (arg-shape refusals are fine, unknown-tool is not).
        let r = tools::call(&db, name, "docs content x");
        if let Err(e) = &r {
            assert!(!e.contains("unknown tool"), "{name} was not dispatched");
        }
    }
}

#[test]
fn report_json_carries_answer_backend_sources_iterations() {
    std::env::set_var("VELESDB_AGENT_URL", "http://127.0.0.1:1");
    let (_dir, db) = demo_db();
    let v = report_json(&run(&db, "what collections exist"));
    assert!(v.get("answer").is_some());
    assert!(v.get("backend").is_some());
    assert!(v.get("sources").is_some());
    assert!(v.get("iterations").is_some());
}

#[test]
fn identifier_arguments_cannot_smuggle_velesql() {
    let (_dir, db) = demo_db();
    for args in [
        "docs;DROP content x",
        "docs content,secret x",
        "docs* content x",
        "docs/**/ content x",
        "1docs content x",
    ] {
        for tool in ["search", "hybrid", "explain"] {
            let e = tools::call(&db, tool, args).unwrap_err();
            assert!(e.contains("invalid"), "{tool}({args}) -> {e}");
        }
    }
    let e = tools::call(&db, "aggregate", "docs a,b").unwrap_err();
    assert!(e.contains("invalid field"), "{e}");
}
