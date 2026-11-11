//! `.agent` REPL controls — offline only (dead `VELESDB_AGENT_URL`).

use super::*;
use tempfile::TempDir;

fn setup() -> (TempDir, Database, ReplConfig) {
    std::env::set_var("VELESDB_AGENT_URL", "http://127.0.0.1:1");
    let dir = TempDir::new().expect("tempdir");
    let db = Database::open(dir.path()).expect("open");
    db.create_collection("docs", 4, velesdb_core::DistanceMetric::Cosine)
        .expect("create docs");
    (dir, db, ReplConfig::default())
}

fn run(db: &Database, config: &mut ReplConfig, line: &str) -> CommandResult {
    let parts: Vec<&str> = line.split_whitespace().collect();
    cmd_agent(db, config, &parts)
}

#[test]
fn tools_and_bare_agent_do_not_run_the_agent() {
    let (_dir, db, mut config) = setup();
    assert!(matches!(
        run(&db, &mut config, "\\agent tools"),
        CommandResult::Continue
    ));
    assert!(matches!(
        run(&db, &mut config, "\\agent"),
        CommandResult::Continue
    ));
    assert!(config.agent.last.is_none(), "no question was asked");
}

#[test]
fn max_iterations_sets_the_session_cap_and_bounds_the_next_run() {
    let (_dir, db, mut config) = setup();
    assert!(matches!(
        run(&db, &mut config, "\\agent max-iterations 1"),
        CommandResult::Continue
    ));
    assert_eq!(config.agent.max_iterations.get(), 1);
    run(&db, &mut config, "\\agent describe docs");
    let report = config.agent.last.as_ref().expect("report kept");
    assert_eq!(
        report.iterations.len(),
        1,
        "cap 1 = only the grounding list()"
    );
}

#[test]
fn max_iterations_rejects_out_of_range_and_keeps_the_previous_cap() {
    let (_dir, db, mut config) = setup();
    for bad in ["0", "33", "-1", "many"] {
        let r = run(&db, &mut config, &format!("\\agent max-iterations {bad}"));
        assert!(
            matches!(&r, CommandResult::Error(e) if e.contains(&format!("1..={}", MaxIterations::LIMIT))),
            "{bad} accepted"
        );
    }
    assert_eq!(config.agent.max_iterations, MaxIterations::default());
}

#[test]
fn save_without_a_run_is_an_error() {
    let (dir, db, mut config) = setup();
    let path = dir.path().join("t.json");
    let r = run(
        &db,
        &mut config,
        &format!("\\agent save {}", path.display()),
    );
    assert!(matches!(r, CommandResult::Error(e) if e.contains("no agent transcript")));
    assert!(!path.exists());
}

#[test]
fn save_writes_the_last_transcript_as_json() {
    let (dir, db, mut config) = setup();
    run(&db, &mut config, "\\agent what collections exist");
    let path = dir.path().join("t.json");
    let r = run(
        &db,
        &mut config,
        &format!("\\agent save {}", path.display()),
    );
    assert!(matches!(r, CommandResult::Continue));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(v["backend"], "offline");
    assert_eq!(v["iterations"][0]["action"], "list()");
    assert!(v["answer"].as_str().is_some_and(|a| a.contains("docs")));
}

#[test]
fn save_to_an_unwritable_path_is_an_error() {
    let (dir, db, mut config) = setup();
    run(&db, &mut config, "\\agent what collections exist");
    let path = dir.path().join("missing-dir").join("t.json");
    let r = run(
        &db,
        &mut config,
        &format!("\\agent save {}", path.display()),
    );
    assert!(matches!(r, CommandResult::Error(e) if e.contains("cannot write")));
}

#[test]
fn other_word_sequences_are_questions() {
    let (_dir, db, mut config) = setup();
    run(&db, &mut config, "\\agent tools for docs please");
    assert!(
        config.agent.last.is_some(),
        "multi-word 'tools …' is a question"
    );
}
