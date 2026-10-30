//! Anti-drift guard: every query sample in the user-facing docs is accepted by
//! the engine.
//!
//! The cheat sheet once described a syntax the parser never accepted, and the
//! guides drifted the same way (MATCH without RETURN, comma joins, `DATE()`,
//! foreign SQL DDL in a `sql` fence). A newcomer copies these samples first,
//! so a rejected one is the worst possible trap. This test walks the
//! user-facing markdown and runs every statement of every ```` ```sql ```` block
//! through [`Parser::parse`] and [`QueryValidator::validate`] (default config),
//! reporting every offender at once with its file and line.
//!
//! # Scope
//!
//! `README.md`, `crates/*/README.md`, and every `*.md` under `docs/`, `sdks/`,
//! `integrations/`, `examples/`, `tutorials/` and `demos/` — except files named
//! `CHANGELOG.md` (history describes past syntax on purpose) and directories
//! that are hidden or named `node_modules` / `target`.
//!
//! # Fences
//!
//! Only a fence whose info string's first word is exactly `sql` is checked.
//! Anything the engine does not accept must use another fence:
//! ```` ```postgresql ```` (or the source's own dialect) for foreign SQL,
//! ```` ```text ```` / ```` ```ebnf ```` for grammar notation, and
//! ```` ```velesql-error ```` for a deliberately rejected example. The last one
//! is checked in reverse: each of its statements must be *rejected*, so a page
//! claiming "this fails" goes red the day the feature lands.
//!
//! # Statement splitting
//!
//! Inside a block, a statement ends at a `;` or at a blank (whitespace-only)
//! line, whichever comes first; `--` starts a comment running to the end of the
//! line. A `;` or `--` inside a `'string'`, `"identifier"` or `` `identifier` ``
//! is literal text. A comment-only line is dropped, not treated as blank, so a
//! comment between two clauses keeps them in one statement. Fragments left
//! empty after this are ignored.

use std::path::{Path, PathBuf};
use velesdb_core::velesql::{Parser, QueryValidator};

/// The fence whose statements must be accepted.
const CHECKED_FENCE: &str = "sql";
/// The fence whose statements must be rejected.
const REJECTED_FENCE: &str = "velesql-error";

const DOC_TREES: [&str; 6] = [
    "docs",
    "sdks",
    "integrations",
    "examples",
    "tutorials",
    "demos",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One statement extracted from a markdown file.
#[derive(Debug, PartialEq)]
struct Sample {
    fence: String,
    /// 1-based line of the statement's first non-blank line.
    line: usize,
    text: String,
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !name.starts_with('.') && name != "node_modules" && name != "target" {
                collect_markdown(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "md") && name != "CHANGELOG.md" {
            out.push(path);
        }
    }
}

/// Every markdown file in scope, sorted for a stable report.
fn doc_files() -> Vec<PathBuf> {
    let root = repo_root();
    let mut files = vec![root.join("README.md")];
    if let Ok(crates) = std::fs::read_dir(root.join("crates")) {
        files.extend(
            crates
                .flatten()
                .map(|entry| entry.path().join("README.md"))
                .filter(|readme| readme.is_file()),
        );
    }
    for tree in DOC_TREES {
        let dir = root.join(tree);
        // A renamed or moved tree must fail here, not drop out of the scan.
        assert!(
            dir.is_dir(),
            "doc tree `{tree}` is missing; update DOC_TREES"
        );
        collect_markdown(&dir, &mut files);
    }
    files.sort();
    files
}

/// Splits the statements out of one fenced block, per the module-doc rule.
struct Splitter {
    fence: String,
    samples: Vec<Sample>,
    current: String,
    start: usize,
}

impl Splitter {
    fn new(fence: &str) -> Self {
        Self {
            fence: fence.to_owned(),
            samples: Vec::new(),
            current: String::new(),
            start: 0,
        }
    }

    fn flush(&mut self) {
        let text = self.current.trim();
        if !text.is_empty() {
            self.samples.push(Sample {
                fence: self.fence.clone(),
                line: self.start,
                text: text.to_owned(),
            });
        }
        self.current.clear();
    }

    fn line(&mut self, line_no: usize, line: &str) {
        if line.trim().is_empty() {
            self.flush();
            return;
        }
        let mut quote: Option<char> = None;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            match quote {
                Some(q) if c == q => quote = None,
                None if matches!(c, '\'' | '"' | '`') => quote = Some(c),
                None if c == '-' && chars.peek() == Some(&'-') => break,
                None if c == ';' => {
                    self.flush();
                    continue;
                }
                _ => {}
            }
            if self.current.trim().is_empty() && !c.is_whitespace() {
                self.start = line_no;
            }
            self.current.push(c);
        }
        self.current.push('\n');
    }
}

/// The first word of a fence's info string, if `line` opens or closes a fence.
fn fence_tag(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("```")?;
    Some(rest.split_whitespace().next().unwrap_or(""))
}

/// Where the scan of a markdown file stands.
enum Fence {
    Outside,
    /// Inside a fence this test does not check.
    Ignored,
    Tracked(Splitter),
}

/// Every statement of every checked or reverse-checked block in `markdown`.
fn samples(markdown: &str) -> Vec<Sample> {
    let mut out = Vec::new();
    let mut fence = Fence::Outside;
    for (idx, line) in markdown.lines().enumerate() {
        match (&mut fence, fence_tag(line)) {
            (Fence::Outside, Some(tag)) if tag == CHECKED_FENCE || tag == REJECTED_FENCE => {
                fence = Fence::Tracked(Splitter::new(tag));
            }
            (Fence::Outside, Some(_)) => fence = Fence::Ignored,
            (Fence::Tracked(splitter), Some(_)) => {
                splitter.flush();
                out.append(&mut splitter.samples);
                fence = Fence::Outside;
            }
            (Fence::Ignored, Some(_)) => fence = Fence::Outside,
            (Fence::Tracked(splitter), None) => splitter.line(idx + 1, line),
            (Fence::Outside | Fence::Ignored, None) => {}
        }
    }
    out
}

/// `Ok` when the engine accepts the statement, the first refusal otherwise.
fn engine_verdict(statement: &str) -> Result<(), String> {
    let query = Parser::parse(statement).map_err(|e| format!("parse: {e}"))?;
    QueryValidator::validate(&query).map_err(|e| format!("validate: {e}"))
}

/// Collapses whitespace runs so each failure reads as one report line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn every_sql_fence_in_user_docs_is_accepted_by_the_engine() {
    let root = repo_root();
    let mut failures = Vec::new();
    let (mut accepted, mut rejected) = (0_usize, 0_usize);
    for file in doc_files() {
        let markdown = std::fs::read_to_string(&file).expect("read markdown file");
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for sample in samples(&markdown) {
            let verdict = engine_verdict(&sample.text);
            let wants_accept = sample.fence == CHECKED_FENCE;
            match (wants_accept, verdict) {
                (true, Ok(())) => accepted += 1,
                (false, Err(_)) => rejected += 1,
                (true, Err(err)) => failures.push(format!(
                    "{rel}:{}: `{}` is rejected: {}",
                    sample.line,
                    one_line(&sample.text),
                    one_line(&err)
                )),
                (false, Ok(())) => failures.push(format!(
                    "{rel}:{}: `{}` is fenced `{REJECTED_FENCE}` but the engine accepts it",
                    sample.line,
                    one_line(&sample.text)
                )),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} doc sample(s) disagree with the engine ({accepted} accepted as documented):\n{}",
        failures.len(),
        failures.join("\n")
    );
    println!("{accepted} doc samples accepted, {rejected} rejected on purpose");
    // Positive control: the walk must actually reach the docs, or an empty
    // scan would pass vacuously.
    assert!(
        accepted >= 500 && rejected > 0,
        "expected the docs to carry hundreds of VelesQL samples, verified {accepted} \
         ({rejected} rejected-on-purpose)"
    );
}

/// A `VelesQL` block under a near-miss tag (`SQL`, `velesql`, ...) would skip
/// the check above without anyone noticing.
#[test]
fn no_velesql_block_hides_under_a_near_miss_fence_tag() {
    let root = repo_root();
    let mut misses = Vec::new();
    for file in doc_files() {
        let markdown = std::fs::read_to_string(&file).expect("read markdown file");
        for (idx, line) in markdown.lines().enumerate() {
            let Some(tag) = fence_tag(line) else { continue };
            let near_miss = (tag.eq_ignore_ascii_case(CHECKED_FENCE) && tag != CHECKED_FENCE)
                || tag.eq_ignore_ascii_case("velesql");
            if near_miss {
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                misses.push(format!("{rel}:{}: fence `{tag}`", idx + 1));
            }
        }
    }
    assert!(
        misses.is_empty(),
        "fence these `{CHECKED_FENCE}` (or `{REJECTED_FENCE}`) so the engine checks them:\n{}",
        misses.join("\n")
    );
}

#[test]
fn cheatsheet_examples_are_all_checked() {
    let path = repo_root().join("docs/reference/VELESQL_CHEATSHEET.md");
    let markdown = std::fs::read_to_string(path).expect("read VELESQL_CHEATSHEET.md");
    let checked: Vec<_> = samples(&markdown)
        .into_iter()
        .filter(|sample| sample.fence == CHECKED_FENCE)
        .collect();
    assert!(
        checked.len() >= 30,
        "expected the cheat sheet to verify many statements, got {}",
        checked.len()
    );
    for sample in checked {
        assert_eq!(
            engine_verdict(&sample.text),
            Ok(()),
            "cheat sheet line {}",
            sample.line
        );
    }
}

#[test]
fn splitting_rule_matches_the_module_doc() {
    let markdown = "\
intro
```sql
SELECT * FROM a -- trailing; comment
-- comment-only line keeps the statement whole
WHERE x = 'a;b--c' LIMIT 1; SELECT * FROM b

SELECT * FROM c
```
```rust
let ignored = 1;
```
```velesql-error
SELECT FROM
```
";
    let got: Vec<(String, usize, String)> = samples(markdown)
        .into_iter()
        .map(|s| (s.fence, s.line, s.text))
        .collect();
    let expected = [
        ("sql", 3, "SELECT * FROM a \n\nWHERE x = 'a;b--c' LIMIT 1"),
        ("sql", 5, "SELECT * FROM b"),
        ("sql", 7, "SELECT * FROM c"),
        (REJECTED_FENCE, 13, "SELECT FROM"),
    ]
    .map(|(fence, line, text)| (fence.to_owned(), line, text.to_owned()));
    assert_eq!(got, expected);
}
