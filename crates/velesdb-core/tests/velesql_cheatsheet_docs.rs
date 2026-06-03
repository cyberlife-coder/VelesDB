//! Anti-drift guard for `docs/reference/VELESQL_CHEATSHEET.md`.
//!
//! The cheat sheet historically described a syntax the parser never accepted,
//! which is the worst possible trap for a newcomer. This test extracts every
//! fenced sql code block from the cheat sheet and asserts each statement parses
//! with the real grammar, so an example can never silently drift again.

use std::path::PathBuf;
use velesdb_core::velesql::Parser;

fn cheatsheet_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/reference/VELESQL_CHEATSHEET.md")
}

/// Collect the contents of every fenced sql code block.
fn sql_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in markdown.lines() {
        match current {
            Some(ref mut buf) => {
                if line.trim_start().starts_with("```") {
                    blocks.push(std::mem::take(buf));
                    current = None;
                } else {
                    buf.push_str(line);
                    buf.push('\n');
                }
            }
            None if line.trim_start().starts_with("```sql") => current = Some(String::new()),
            None => {}
        }
    }
    blocks
}

/// Split a block into individual statements. Line comments are stripped first
/// (a `--` comment may itself contain a `;`, which must not split a statement).
fn statements(block: &str) -> Vec<String> {
    let without_comments = block
        .lines()
        .map(|line| line.find("--").map_or(line, |idx| &line[..idx]))
        .collect::<Vec<_>>()
        .join("\n");
    without_comments
        .split(';')
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .map(String::from)
        .collect()
}

#[test]
fn cheatsheet_sql_examples_all_parse() {
    let markdown = std::fs::read_to_string(cheatsheet_path()).expect("read VELESQL_CHEATSHEET.md");
    let blocks = sql_blocks(&markdown);
    assert!(!blocks.is_empty(), "no ```sql blocks found in cheat sheet");

    let mut verified = 0_usize;
    for block in &blocks {
        for statement in statements(block) {
            let result = Parser::parse(&statement);
            assert!(
                result.is_ok(),
                "cheat sheet example failed to parse:\n  {statement}\n  error: {:?}",
                result.err()
            );
            verified += 1;
        }
    }

    assert!(
        verified >= 30,
        "expected the cheat sheet to verify many statements, got {verified}"
    );
}
