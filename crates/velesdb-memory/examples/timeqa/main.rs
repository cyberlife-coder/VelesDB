//! `TimeQA` filtered-recall — REAL-data `ColumnStore` (`recall_where` year range) test.
//!
//! Each `TimeQA` question scopes a time window over one person's Wikipedia bio
//! ("What was the position of X from 1997 to 2001?"). The bio lists the person's
//! many time-stamped positions, so a vector retriever finds them all (they look
//! alike) and cannot pick the period; `recall_where(year ≥ lo AND year ≤ hi)`
//! filters by the numeric year column. Generation-free: we score whether the
//! *sentence(s) containing the gold answer* are retrieved, vector-only vs
//! vector + `ColumnStore` filter.
//!
//! Run `--validate` FIRST to hand-check the year/range/gold parsing before
//! trusting any ablation (a parsing bug would be misattributed to the engine).
//!
//! ```text
//! cargo run --release -p velesdb-memory --features ollama --example timeqa -- --validate
//! cargo run --release -p velesdb-memory --features ollama --example timeqa -- --k 5 --embed-model mxbai-embed-large
//! ```

use std::error::Error;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::{Map, Value};
use velesdb_memory::mcp::DynEmbedder;
use velesdb_memory::{ColumnFilter, ColumnOp, MemoryService, OllamaEmbedder, DEFAULT_OLLAMA_URL};

// benchmark harness: field name mirrors dataset JSON key
#[allow(clippy::struct_field_names)]
#[derive(Deserialize)]
struct Question {
    question: String,
    targets: Vec<String>,
    context: String,
}

struct Args {
    dataset: PathBuf,
    k: usize,
    embed: String,
    n: usize,
    validate: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let data: Vec<Question> = serde_json::from_slice(&std::fs::read(&args.dataset)?)?;
    let take = args.n.min(data.len());
    if args.validate {
        return validate(&data, take.min(8));
    }
    let mut report = Report::default();
    for (pos, q) in data.iter().take(take).enumerate() {
        let Some((lo, hi)) = question_range(&q.question) else {
            continue;
        };
        let golds: Vec<&str> = q
            .targets
            .iter()
            .map(String::as_str)
            .filter(|t| !t.trim().is_empty())
            .collect();
        if golds.is_empty() {
            continue; // unanswerable for this period — not a retrieval target
        }
        let store = build(q, &args, pos)?;
        let gold_ids = store.gold_ids(&golds);
        if gold_ids.is_empty() {
            continue; // answer text never matched a sentence — skip (not engine's fault)
        }
        let vector = recall_ids(&store.svc.recall(&q.question, args.k, None)?);
        let filters = vec![
            ColumnFilter {
                field: "year".into(),
                op: ColumnOp::Ge,
                value: Value::from(lo),
            },
            ColumnFilter {
                field: "year".into(),
                op: ColumnOp::Le,
                value: Value::from(hi),
            },
        ];
        let filtered = recall_ids(&store.svc.recall_where(&q.question, args.k, &filters)?);
        report.record(recall(&vector, &gold_ids), recall(&filtered, &gold_ids));
        drop(store);
        cleanup(pos);
        if (pos + 1) % 50 == 0 {
            eprintln!("  {}/{} questions", pos + 1, take);
        }
    }
    report.print(&args);
    Ok(())
}

/// Print parsed range, sentence/year counts, and the gold sentences for a few
/// questions — to hand-verify the parsing before running the ablation.
// benchmark harness: Result kept for call-site `?` symmetry
#[allow(clippy::unnecessary_wraps)]
fn validate(data: &[Question], n: usize) -> Result<(), Box<dyn Error>> {
    println!("=== TimeQA parsing validation (hand-check before trusting numbers) ===\n");
    for q in data.iter().take(n) {
        let range = question_range(&q.question);
        let golds: Vec<&str> = q
            .targets
            .iter()
            .map(String::as_str)
            .filter(|t| !t.trim().is_empty())
            .collect();
        let sents = dated_sentences(&q.context);
        let with_year = sents.iter().filter(|(_, y)| y.is_some()).count();
        println!("Q: {}", q.question);
        println!("  parsed range: {range:?}   targets: {golds:?}");
        println!(
            "  sentences: {} ({with_year} dated after imputation)",
            sents.len()
        );
        for (s, y) in &sents {
            if golds.iter().any(|g| contains(s, g)) {
                let inrange =
                    range.is_some_and(|(lo, hi)| y.is_some_and(|yy| yy >= lo && yy <= hi));
                println!(
                    "  GOLD[year={y:?} in-range={inrange}]: {}",
                    truncate(s, 100)
                );
            }
        }
        println!();
    }
    Ok(())
}

struct Store {
    svc: MemoryService<DynEmbedder>,
    sentences: Vec<(u64, String)>,
}

impl Store {
    /// Sentence ids whose text contains any gold answer span.
    fn gold_ids(&self, golds: &[&str]) -> Vec<u64> {
        self.sentences
            .iter()
            .filter(|(_, s)| golds.iter().any(|g| contains(s, g)))
            .map(|(id, _)| *id)
            .collect()
    }
}

fn recall_ids(hits: &[velesdb_memory::Recollection]) -> Vec<u64> {
    hits.iter().map(|h| h.id).collect()
}

fn recall(retrieved: &[u64], gold: &[u64]) -> f64 {
    let hit = gold.iter().filter(|g| retrieved.contains(g)).count();
    if gold.is_empty() {
        0.0
    } else {
        f64::from(u32::try_from(hit).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(gold.len()).unwrap_or(u32::MAX))
    }
}

/// Ingest a question's bio: one memory per sentence, with its first year as the
/// numeric `year` `ColumnStore` facet (sentences with no year carry no facet, so
/// `recall_where` excludes them — disclosed).
fn build(q: &Question, args: &Args, pos: usize) -> Result<Store, Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("velesdb-timeqa-{}-{pos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let embedder: DynEmbedder = Box::new(OllamaEmbedder::new(DEFAULT_OLLAMA_URL, &args.embed)?);
    let svc = MemoryService::open(dir, embedder)?;
    let mut sentences_idx = Vec::new();
    for (s, year) in dated_sentences(&q.context) {
        let meta = year.map(|y| {
            let mut m = Map::new();
            m.insert("year".into(), Value::from(y));
            m
        });
        let id = svc.remember(&s, &[], meta.as_ref())?;
        sentences_idx.push((id, s));
    }
    Ok(Store {
        svc,
        sentences: sentences_idx,
    })
}

/// Split context into sentences (period/newline boundaries), keeping substantive
/// ones.
fn sentences(ctx: &str) -> Vec<String> {
    ctx.replace('\n', " ")
        .split(". ")
        .map(|s| s.trim().to_string())
        .filter(|s| s.len() >= 25)
        .collect()
}

/// Sentences paired with a year, forward-filling the last-seen year onto an
/// undated sentence — bios are roughly chronological, and the answer sentence
/// ("became Minister") often follows the dated one ("In 1996…") without repeating
/// the year. This is a heuristic imputation; it is disclosed.
fn dated_sentences(ctx: &str) -> Vec<(String, Option<i64>)> {
    let mut last: Option<i64> = None;
    sentences(ctx)
        .into_iter()
        .map(|s| {
            let found = sentence_year(&s);
            let y = found.or(last);
            if let Some(f) = found {
                last = Some(f);
            }
            (s, y)
        })
        .collect()
}

/// All 4-digit calendar years (1600-2099) in `text`, in order.
fn years(text: &str) -> Vec<i64> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= b.len() {
        if b[i].is_ascii_digit()
            && b[i + 1].is_ascii_digit()
            && b[i + 2].is_ascii_digit()
            && b[i + 3].is_ascii_digit()
            && !(i > 0 && b[i - 1].is_ascii_digit())
            && !(i + 4 < b.len() && b[i + 4].is_ascii_digit())
        {
            let y = (i64::from(b[i] - b'0')) * 1000
                + i64::from(b[i + 1] - b'0') * 100
                + i64::from(b[i + 2] - b'0') * 10
                + i64::from(b[i + 3] - b'0');
            if (1600..=2099).contains(&y) {
                out.push(y);
            }
            i += 4;
        } else {
            i += 1;
        }
    }
    out
}

/// The question's scoped window `[lo, hi]` (first..last year named), if any.
fn question_range(q: &str) -> Option<(i64, i64)> {
    let ys = years(q);
    let lo = *ys.iter().min()?;
    let hi = *ys.iter().max()?;
    Some((lo, hi))
}

/// A sentence's first year (its `ColumnStore` facet), if any.
fn sentence_year(s: &str) -> Option<i64> {
    years(s).into_iter().next()
}

/// Normalised case-insensitive substring containment (collapses whitespace).
fn contains(haystack: &str, needle: &str) -> bool {
    let norm = |s: &str| {
        s.to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let n = norm(needle);
    !n.is_empty() && norm(haystack).contains(&n)
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn cleanup(pos: usize) {
    let _ = std::fs::remove_dir_all(
        std::env::temp_dir().join(format!("velesdb-timeqa-{}-{pos}", std::process::id())),
    );
}

#[derive(Default)]
struct Report {
    n: u32,
    vector: f64,
    filtered: f64,
}

impl Report {
    fn record(&mut self, vector: f64, filtered: f64) {
        self.n += 1;
        self.vector += vector;
        self.filtered += filtered;
    }

    fn print(&self, args: &Args) {
        let vr = pct(self.vector, self.n);
        let fr = pct(self.filtered, self.n);
        println!("\nVelesDB-memory — TimeQA filtered-recall (real-data, generation-free)");
        println!(
            "embedder: ollama / {} · {} answerable time-scoped questions · k={}\n",
            args.embed, self.n, args.k
        );
        println!("  gold-sentence recall@k:");
        println!("    vector only          {vr:.1}%");
        println!("    + ColumnStore filter {fr:.1}%   ({:+.1} pp)", fr - vr);
        println!("  (recall = share of answer-bearing sentences in the top-k; filter = recall_where year>=lo AND year<=hi)");
    }
}

fn pct(sum: f64, n: u32) -> f64 {
    if n == 0 {
        0.0
    } else {
        100.0 * sum / f64::from(n)
    }
}

impl Args {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut args = Args {
            dataset: manifest.join("examples/timeqa/data/timeqa_val_500.json"),
            k: 5,
            embed: "mxbai-embed-large".to_string(),
            n: usize::MAX,
            validate: false,
        };
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < raw.len() {
            let flag = &raw[i];
            if flag == "--validate" {
                args.validate = true;
                i += 1;
                continue;
            }
            let val = raw
                .get(i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--dataset" => args.dataset = PathBuf::from(val),
                "--k" => args.k = val.parse()?,
                "--embed-model" => args.embed.clone_from(val),
                "--questions" => args.n = val.parse()?,
                other => return Err(format!("unknown argument: {other}").into()),
            }
            i += 2;
        }
        Ok(args)
    }
}
