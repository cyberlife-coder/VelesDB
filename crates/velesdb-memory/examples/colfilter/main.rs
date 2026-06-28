//! Filtered-recall pilot — does the `ColumnStore` (`recall_where`, numeric year
//! range) beat time-blind vector recall when the answer depends on a time scope?
//!
//! Generation-free and fully synthetic, so it isolates the *engine* from any
//! extraction noise. We mint (person, role, org, year) tuples: each `(org, role)`
//! has a succession of office-holders across the years, so every tuple for one
//! `(org, role)` is near-identical in embedding space and differs only by its
//! **year** — a metadata column. A question scopes a year window; the gold answers
//! are the holders inside it. A time-blind vector retriever cannot disambiguate
//! (all holders of the role look alike); `recall_where(year ≥ lo AND year ≤ hi)`
//! filters by the column. Two arms: vector-only vs vector + `ColumnStore` filter.
//! (The graph leg is exercised separately by the `triengine` example, which stacks
//! all three engines.) Metric: answer-bearing recall@k, split by the *hard* subset
//! (≥2 in-window answers, where the range predicate is genuinely required) vs the
//! single-answer subset (where vector already suffices).
//!
//! ```text
//! cargo run --release -p velesdb-memory --features ollama --example colfilter -- \
//!   --embed-model mxbai-embed-large --k 5
//! ```

use std::error::Error;

use serde_json::{Map, Value};
use velesdb_memory::DynEmbedder;
use velesdb_memory::{ColumnFilter, ColumnOp, MemoryService, OllamaEmbedder, DEFAULT_OLLAMA_URL};

const ORGS: &[&str] = &[
    "Acme Corp",
    "Globex",
    "Initech",
    "Umbrella",
    "Stark Industries",
    "Wayne Enterprises",
    "Wonka Industries",
    "Cyberdyne",
    "Soylent",
    "Hooli",
    "Pied Piper",
    "Massive Dynamic",
];
const ROLES: &[&str] = &[
    "chief executive officer",
    "chief technology officer",
    "chief financial officer",
];
const START_YEAR: i64 = 2000;
const TENURE: i64 = 3; // a new holder every 3 years
const HOLDERS: i64 = 7; // 7 holders per (org, role) → 2000..2018

struct Args {
    k: usize,
    embed: String,
}

/// One synthetic memory: the (org, role, year) it states, kept to score recall.
struct Fact {
    org: usize,
    role: usize,
    year: i64,
    id: u64,
}

/// A scoped question over one (org, role): "who held it between lo and hi?".
struct Probe {
    org: usize,
    role: usize,
    lo: i64,
    hi: i64,
    gold: Vec<u64>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let (svc, facts) = build(&args)?;
    let probes = probes(&facts);
    let mut report = Report::default();
    for p in &probes {
        let question = format!(
            "Who was the {} of {} between {} and {}?",
            ROLES[p.role], ORGS[p.org], p.lo, p.hi
        );
        let vector = recall_ids(&svc.recall(&question, args.k, None)?);
        let filters = vec![
            ColumnFilter {
                field: "year".into(),
                op: ColumnOp::Ge,
                value: Value::from(p.lo),
            },
            ColumnFilter {
                field: "year".into(),
                op: ColumnOp::Le,
                value: Value::from(p.hi),
            },
        ];
        let filtered = recall_ids(&svc.recall_where(&question, args.k, &filters)?);
        report.record(
            p.gold.len(),
            recall(&vector, &p.gold),
            recall(&filtered, &p.gold),
        );
    }
    report.print(&args, probes.len());
    Ok(())
}

/// The memory ids of a recollection list.
fn recall_ids(hits: &[velesdb_memory::Recollection]) -> Vec<u64> {
    hits.iter().map(|h| h.id).collect()
}

/// Answer-bearing recall: fraction of `gold` ids present in `retrieved`.
fn recall(retrieved: &[u64], gold: &[u64]) -> f64 {
    let hit = gold.iter().filter(|g| retrieved.contains(g)).count();
    if gold.is_empty() {
        0.0
    } else {
        f64::from(u32::try_from(hit).unwrap_or(u32::MAX))
            / f64::from(u32::try_from(gold.len()).unwrap_or(u32::MAX))
    }
}

/// Build the synthetic store: one dated memory per (org, role, holder).
fn build(args: &Args) -> Result<(MemoryService<DynEmbedder>, Vec<Fact>), Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("velesdb-colfilter-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let embedder: DynEmbedder = Box::new(OllamaEmbedder::new(DEFAULT_OLLAMA_URL, &args.embed)?);
    let svc = MemoryService::open(dir, embedder)?;
    let mut facts = Vec::new();
    // benchmark harness: indices double as stored Fact fields
    #[allow(clippy::needless_range_loop)]
    for org in 0..ORGS.len() {
        for role in 0..ROLES.len() {
            for h in 0..HOLDERS {
                let year = START_YEAR + h * TENURE;
                let person = format!("{}_{}_{h}", short(ORGS[org]), short_role(ROLES[role]));
                let text = format!(
                    "{person} served as the {} of {} in {year}.",
                    ROLES[role], ORGS[org]
                );
                let mut meta = Map::new();
                meta.insert("year".into(), Value::from(year));
                let id = svc.remember(&text, &[], Some(&meta))?;
                facts.push(Fact {
                    org,
                    role,
                    year,
                    id,
                });
            }
        }
    }
    Ok((svc, facts))
}

/// One scoped probe per (org, role) per window width {2-year, 6-year, 12-year},
/// so the set spans single-answer and multi-answer (hard) questions.
fn probes(facts: &[Fact]) -> Vec<Probe> {
    let mut out = Vec::new();
    let mid = START_YEAR + (HOLDERS / 2) * TENURE;
    for org in 0..ORGS.len() {
        for role in 0..ROLES.len() {
            for half in [1, 3, 6] {
                let (lo, hi) = (mid - half, mid + half);
                let gold: Vec<u64> = facts
                    .iter()
                    .filter(|f| f.org == org && f.role == role && f.year >= lo && f.year <= hi)
                    .map(|f| f.id)
                    .collect();
                if !gold.is_empty() {
                    out.push(Probe {
                        org,
                        role,
                        lo,
                        hi,
                        gold,
                    });
                }
            }
        }
    }
    out
}

fn short(org: &str) -> String {
    org.split_whitespace().next().unwrap_or(org).to_string()
}
fn short_role(role: &str) -> String {
    role.split_whitespace()
        .map(|w| w.chars().next().unwrap_or('x'))
        .collect()
}

#[derive(Default)]
struct Report {
    hard: Cell,
    single: Cell,
}

#[derive(Default, Clone, Copy)]
struct Cell {
    n: u32,
    vector: f64,
    filtered: f64,
}

impl Report {
    fn record(&mut self, gold: usize, vector: f64, filtered: f64) {
        let cell = if gold >= 2 {
            &mut self.hard
        } else {
            &mut self.single
        };
        cell.n += 1;
        cell.vector += vector;
        cell.filtered += filtered;
    }

    fn print(&self, args: &Args, n: usize) {
        println!("\nVelesDB-memory — ColumnStore filtered-recall pilot (numeric year range)");
        println!(
            "embedder: ollama / {} · {n} time-scoped probes · k={} · synthetic, generation-free\n",
            args.embed, args.k
        );
        println!(
            "  {:<26}{:>5}   {:>12} {:>14}",
            "subset", "n", "vector-rec", "+recall_where"
        );
        self.row("hard (>=2 in-window)", self.hard);
        self.row("single (1 in-window)", self.single);
        let all = Cell {
            n: self.hard.n + self.single.n,
            vector: self.hard.vector + self.single.vector,
            filtered: self.hard.filtered + self.single.filtered,
        };
        self.row("ALL", all);
        let vr = pct(self.hard.vector, self.hard.n);
        let fr = pct(self.hard.filtered, self.hard.n);
        println!(
            "\n→ HARD subset (range predicate genuinely required): vector {vr:.1}% → +ColumnStore {fr:.1}% ({:+.1} pp)",
            fr - vr
        );
        println!("  (recall = share of in-window gold tuples in the top-k; ColumnStore arm adds recall_where year>=lo AND year<=hi)");
    }

    #[allow(clippy::unused_self)]
    fn row(&self, label: &str, c: Cell) {
        println!(
            "  {:<26}{:>5}   {:>11.1}% {:>13.1}%",
            label,
            c.n,
            pct(c.vector, c.n),
            pct(c.filtered, c.n)
        );
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
        let mut args = Args {
            k: 5,
            embed: "mxbai-embed-large".to_string(),
        };
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < raw.len() {
            let flag = &raw[i];
            let val = raw
                .get(i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--k" => args.k = val.parse()?,
                "--embed-model" => args.embed.clone_from(val),
                other => return Err(format!("unknown argument: {other}").into()),
            }
            i += 2;
        }
        Ok(args)
    }
}
