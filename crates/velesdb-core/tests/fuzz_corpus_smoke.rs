//! Regression guard over the committed fuzz corpus (PR-time smoke).
//!
//! The mutational fuzzers run nightly only (`quality-deep.yml`); this test
//! runs on every PR and replays the committed parser corpus through
//! `Parser::parse`, so any input the fuzzer ever reduced into the corpus —
//! or any hand-written hostile seed — becomes a permanent no-panic
//! regression test on the stable toolchain, with no cargo-fuzz or nightly
//! in the PR path. `Parser::parse` must never panic: returning an error is
//! the only acceptable failure mode for arbitrary input.

use std::path::PathBuf;

fn parser_corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/fuzz_velesql_parser")
}

#[test]
fn every_committed_parser_corpus_seed_parses_without_panicking() {
    let dir = parser_corpus_dir();
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("fuzz corpus dir {} must exist: {e}", dir.display()))
        .filter_map(Result::ok)
        .filter(|e| e.path().is_file())
        .collect();
    assert!(
        entries.len() >= 20,
        "the committed parser corpus shrank to {} seeds — it should only grow",
        entries.len()
    );

    for entry in entries {
        let bytes = std::fs::read(entry.path()).expect("seed must be readable");
        if let Ok(input) = std::str::from_utf8(&bytes) {
            // Mirror of the fuzz target's contract: parse may err, never panic.
            let _ = velesdb_core::velesql::Parser::parse(input);
        }
    }
}
