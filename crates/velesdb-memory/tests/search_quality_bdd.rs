//! Recall at a configured quality.
//!
//! The quality is the operator's recall/latency dial. Whatever it is set to,
//! recall must still find what was remembered, and a bad setting must stop the
//! daemon instead of being quietly ignored.

#![cfg(feature = "persistence")]

mod common;

use common::{service, DIM};
use tempfile::TempDir;
use velesdb_memory::{HashEmbedder, MemoryService, SearchQuality};

const FACT: &str = "we chose parking_lot to avoid lock poisoning";

fn service_at(quality: SearchQuality) -> (TempDir, MemoryService<HashEmbedder>) {
    let dir = TempDir::new().expect("create tempdir");
    let svc = MemoryService::open(dir.path(), HashEmbedder::new(DIM))
        .expect("open memory store")
        .with_search_quality(quality);
    (dir, svc)
}

#[test]
fn recall_finds_a_remembered_fact_at_every_quality() {
    for quality in [
        SearchQuality::Fast,
        SearchQuality::Balanced,
        SearchQuality::Accurate,
        SearchQuality::Perfect,
        SearchQuality::AutoTune,
        SearchQuality::Custom(128),
    ] {
        let (_dir, svc) = service_at(quality);
        let id = svc.remember(FACT, &[], None).expect("remember");
        svc.remember("the weather is mild today", &[], None)
            .expect("remember a distractor");

        let hits = svc
            .recall("parking_lot lock poisoning", 5, None)
            .expect("recall");

        assert_eq!(
            hits.first().map(|hit| hit.id),
            Some(id),
            "{quality:?} must rank the stored fact first"
        );
    }
}

#[test]
fn a_configured_quality_ranks_like_the_default() {
    let (_default_dir, default) = service();
    let (_perfect_dir, perfect) = service_at(SearchQuality::Perfect);
    for text in [FACT, "the weather is mild today", "tests run in CI"] {
        default.remember(text, &[], None).expect("remember");
        perfect.remember(text, &[], None).expect("remember");
    }

    let texts = |svc: &MemoryService<HashEmbedder>| -> Vec<String> {
        svc.recall("parking_lot lock poisoning", 3, None)
            .expect("recall")
            .into_iter()
            .map(|hit| hit.content)
            .collect()
    };

    assert_eq!(texts(&perfect), texts(&default));
}

// --- The daemon -------------------------------------------------------------

/// Run the real binary against a scratch store with `quality` in the
/// environment and return how it ended: `None` if it was still running after
/// `grace` (killed), else its exit success and stderr.
fn run_daemon(quality: &str, grace: std::time::Duration) -> Option<(bool, String)> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let store = TempDir::new().expect("scratch store");
    let mut child = Command::new(env!("CARGO_BIN_EXE_velesdb-memory"))
        .arg("--http")
        .arg("--http-insecure")
        .arg("--http-port")
        .arg("0")
        .env("VELESDB_MEMORY_PATH", store.path())
        .env("VELESDB_MEMORY_SEARCH_QUALITY", quality)
        .env_remove("VELESDB_MEMORY_CONFIG")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn velesdb-memory");
    let mut stderr = child.stderr.take().expect("piped stderr");

    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll the daemon") {
            let mut out = String::new();
            let _ = stderr.read_to_string(&mut out);
            return Some((status.success(), out));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    None
}

#[test]
fn a_search_quality_that_is_not_one_stops_the_daemon_naming_the_setting() {
    let (succeeded, stderr) = run_daemon("turbo", std::time::Duration::from_secs(10))
        .expect("the daemon must refuse, not run");

    assert!(!succeeded, "a bad quality must be a failed start");
    assert!(
        stderr.contains("VELESDB_MEMORY_SEARCH_QUALITY") && stderr.contains("turbo"),
        "the operator must see which setting and which value: {stderr}"
    );
}

#[test]
fn a_valid_search_quality_is_not_what_stops_the_daemon() {
    // Whether the daemon is still serving after the grace period or has ended
    // for a reason of its own (this test does not care which), it must not have
    // refused the setting.
    if let Some((_, stderr)) = run_daemon("accurate", std::time::Duration::from_secs(3)) {
        assert!(
            !stderr.contains("VELESDB_MEMORY_SEARCH_QUALITY"),
            "a valid quality must not be refused: {stderr}"
        );
    }
}
