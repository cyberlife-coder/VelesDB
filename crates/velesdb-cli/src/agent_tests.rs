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
fn tool_identifier_arguments_cannot_smuggle_velesql() {
    let (_dir, db) = demo_db();
    // Bad input in the collection slot — all four SQL-building tools refuse.
    for args in [
        "docs;DROP content x",
        "docs' content x",
        "docs/**/ content x",
        "docs,tmp content x",
        "1docs content x",
    ] {
        for tool in ["search", "hybrid", "explain", "aggregate"] {
            let e = tools::call(&db, tool, args).unwrap_err();
            assert!(e.contains("invalid"), "{tool}({args}) -> {e}");
        }
    }
    // Bad input in the field slot refuses too.
    for args in ["docs content;DROP x", "docs a,b x"] {
        for tool in ["search", "hybrid", "explain", "aggregate"] {
            let e = tools::call(&db, tool, args).unwrap_err();
            assert!(e.contains("invalid"), "{tool}({args}) -> {e}");
        }
    }
}

/// Fake Ollama: the first accepted connection gets `tags_body` with 200,
/// every later connection gets `generate_status` with `{}`. Returns the port.
fn fake_ollama(tags_body: &'static str, generate_status: &'static str) -> u16 {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || {
        let mut first = true;
        while let Ok((mut s, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf);
            let (status, body) = if first {
                first = false;
                ("200 OK", tags_body)
            } else {
                (generate_status, "{}")
            };
            let _ = s.write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    port
}

fn agent_env(url_port: u16, model: &str) {
    std::env::set_var("VELESDB_AGENT_URL", format!("http://127.0.0.1:{url_port}"));
    std::env::set_var("VELESDB_AGENT_MODEL", model);
}

fn clear_agent_env() {
    std::env::remove_var("VELESDB_AGENT_URL");
    std::env::remove_var("VELESDB_AGENT_MODEL");
}

#[test]
fn model_pulled_matches_tagged_and_latest_names() {
    let tags = serde_json::json!([
        {"name": "qwen3:14b"},
        {"name": "llama3.1:latest"}
    ]);
    let models = tags.as_array().expect("array");
    assert!(llm::model_pulled(models, "qwen3:14b"));
    assert!(llm::model_pulled(models, "llama3.1")); // untagged -> :latest
    assert!(!llm::model_pulled(models, "qwen3:4b"));
    assert!(!llm::model_pulled(models, "other"));
}

#[test]
fn backend_returns_none_when_the_model_is_not_pulled() {
    let port = fake_ollama(r#"{"models":[{"name":"qwen3:14b"}]}"#, "200 OK");
    agent_env(port, "no-such-model:9b");
    assert!(
        llm::backend().is_none(),
        "absent model must not select ollama"
    );
    clear_agent_env();
}

#[test]
fn generate_404_falls_back_to_the_offline_router() {
    // tags lists the model (probe passes), /api/generate then 404s — the
    // documented gap: the run must still answer via the offline router.
    let port = fake_ollama(r#"{"models":[{"name":"m:1"}]}"#, "404 Not Found");
    agent_env(port, "m:1");
    let (_dir, db) = demo_db();
    let report = run(&db, "what collections exist");
    assert!(
        report.backend.starts_with("offline"),
        "expected offline fallback, got backend={}",
        report.backend
    );
    assert!(report.backend.contains("404"), "backend hides the cause");
    assert!(
        report.answer.contains("docs"),
        "offline router still answered"
    );
    clear_agent_env();
}
