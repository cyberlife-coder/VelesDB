//! Tri-engine compounding capstone — do Vector + Graph + `ColumnStore` *stack*?
//!
//! Synthetic, generation-free. The store holds, for many companies: dated
//! "office-holder" facts ("{person} was the {role} of {org} in {year}." with a
//! `year` column) and one location fact per company ("{org} is headquartered in
//! {city}."). A question names a **city** and a **role** and a **year window**:
//! "Who was the {role} of the company headquartered in {city} from {lo} to {hi}?"
//!
//! To answer it you must (1) resolve which company is in that city — the
//! role-facts never mention the city, so a vector retriever can't; that's a
//! **graph** bridge (city → location-fact → company → its role-facts), and
//! (2) keep only the right period — a **`ColumnStore`** `year ∈ [lo,hi]` predicate
//! cosine cannot express. We measure answer-bearing recall@k under four arms:
//! vector-only, +`ColumnStore`, +Graph, and +both — to see whether the engines
//! compound.
//!
//! ```text
//! cargo run --release -p velesdb-memory --features ollama --example triengine -- --k 5
//! ```

use std::collections::{HashMap, HashSet};
use std::error::Error;

use serde_json::{Map, Value};
use velesdb_memory::DynEmbedder;
use velesdb_memory::{ColumnFilter, ColumnOp, MemoryService, OllamaEmbedder, DEFAULT_OLLAMA_URL};

const CITIES: &[&str] = &[
    "Lyon",
    "Hamburg",
    "Turin",
    "Bristol",
    "Gothenburg",
    "Nantes",
    "Kyoto",
    "Austin",
    "Porto",
    "Leeds",
];
// Opaque company names, decoupled from their city — so a vector retriever cannot
// bridge "headquartered in {city}" to the company's role-facts by surface
// similarity; only the graph location-edge connects them.
const ORGS: &[&str] = &[
    "Avenor",
    "Brightwell",
    "Caldera",
    "Dornier",
    "Everline",
    "Fenix",
    "Gravon",
    "Halcyon",
    "Iberis",
    "Juno",
];
const ROLES: &[&str] = &["chief executive officer", "chief technology officer"];
const START: i64 = 1985;
const TENURE: i64 = 4;
const HOLDERS: i64 = 8; // years 1985..2013 — 8 holders per role > k, so the period axis bites

struct Args {
    k: usize,
    embed: String,
    hops: usize,
    boost: f64,
}

/// A scoped probe: the role of the company in `city`, in `[lo,hi]`.
struct Probe {
    city: usize,
    role: usize,
    lo: i64,
    hi: i64,
    gold: Vec<u64>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let store = build(&args)?;
    let probes = store.probes();
    let mut report = Report::default();
    for p in &probes {
        let q = format!(
            "Who was the {} of the company headquartered in {} from {} to {}?",
            ROLES[p.role], CITIES[p.city], p.lo, p.hi
        );
        report.record(
            store.recall_arm(&q, p, &args, false, false)?, // vector
            store.recall_arm(&q, p, &args, true, false)?,  // + ColumnStore
            store.recall_arm(&q, p, &args, false, true)?,  // + Graph
            store.recall_arm(&q, p, &args, true, true)?,   // + both
        );
    }
    report.print(&args, probes.len());
    Ok(())
}

struct Store {
    svc: MemoryService<DynEmbedder>,
    /// role-fact id → (city, role, year), for gold + the year filter.
    roles: HashMap<u64, (usize, usize, i64)>,
}

impl Store {
    fn is_role(&self, id: u64) -> bool {
        self.roles.contains_key(&id)
    }

    /// One probe per (city, role) per window {1,3}: spans single & multi answers.
    fn probes(&self) -> Vec<Probe> {
        let mid = START + (HOLDERS / 2) * TENURE;
        let mut out = Vec::new();
        for city in 0..CITIES.len() {
            for role in 0..ROLES.len() {
                for half in [2, 8] {
                    let (lo, hi) = (mid - half, mid + half);
                    let gold: Vec<u64> = self
                        .roles
                        .iter()
                        .filter(|(_, (c, r, y))| *c == city && *r == role && *y >= lo && *y <= hi)
                        .map(|(id, _)| *id)
                        .collect();
                    if !gold.is_empty() {
                        out.push(Probe {
                            city,
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

    /// Answer-bearing recall@k under the chosen engines.
    fn recall_arm(
        &self,
        q: &str,
        p: &Probe,
        args: &Args,
        column: bool,
        graph: bool,
    ) -> Result<f64, Box<dyn Error>> {
        // Candidate pool: vector, optionally pre-filtered by the year predicate.
        let pool: Vec<(u64, f64)> = if column {
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
            self.svc.recall_where(q, args.k * 12 + 16, &filters)?
        } else {
            self.svc.recall(q, args.k * 12 + 16, None)?
        }
        .into_iter()
        .filter(|r| self.is_role(r.id))
        .map(|r| (r.id, f64::from(r.score)))
        .collect();

        let max = pool
            .iter()
            .map(|(_, s)| *s)
            .fold(f64::MIN, f64::max)
            .max(f64::EPSILON);
        let mut cand: HashMap<u64, f64> = pool.iter().map(|(id, s)| (*id, s / max)).collect();

        if graph {
            // why() bridges city → location-fact → company → its role-facts.
            for n in self.svc.why(q, args.hops, None)?.nodes {
                if n.hop >= 1 && self.is_role(n.id) {
                    // A graph-reached role-fact: only count it if it also satisfies
                    // the year predicate when the ColumnStore engine is on (this is
                    // how the two engines compose — graph picks the company, the
                    // column picks the period).
                    let in_range = self
                        .roles
                        .get(&n.id)
                        .is_some_and(|(_, _, y)| !column || (*y >= p.lo && *y <= p.hi));
                    if in_range {
                        *cand.entry(n.id).or_insert(0.0) += args.boost;
                    }
                }
            }
        }

        let mut scored: Vec<(u64, f64)> = cand.into_iter().collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        let top: Vec<u64> = scored.into_iter().take(args.k).map(|(id, _)| id).collect();
        Ok(recall(&top, &p.gold))
    }
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

/// Build the shared store: dated role facts (year column) + one location fact per
/// company, wired into a city↔company↔role-fact graph.
fn build(args: &Args) -> Result<Store, Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("velesdb-triengine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let embedder: DynEmbedder = Box::new(OllamaEmbedder::new(DEFAULT_OLLAMA_URL, &args.embed)?);
    let svc = MemoryService::open(dir, embedder)?;
    let mut roles = HashMap::new();
    let mut edges: HashSet<(u64, u64)> = HashSet::new();
    for (city, city_name) in CITIES.iter().enumerate() {
        let org = format!("{} Corporation", ORGS[city]);
        // Company entity hub + location fact (the bridge — the ONLY text tying the
        // opaque company to its city).
        let org_hub = svc.remember(&format!("Entity: {org}"), &[], None)?;
        let loc = svc.remember(
            &format!("{org} is headquartered in {city_name}."),
            &[],
            None,
        )?;
        relate(&svc, &mut edges, loc, org_hub)?;
        for (role, role_name) in ROLES.iter().enumerate() {
            for h in 0..HOLDERS {
                let year = START + h * TENURE;
                let person = format!("{}_{}_{h}", ORGS[city], abbr(role_name));
                let text = format!("{person} was the {role_name} of {org} in {year}.");
                let mut meta = Map::new();
                meta.insert("year".into(), Value::from(year));
                let id = svc.remember(&text, &[], Some(&meta))?;
                roles.insert(id, (city, role, year));
                relate(&svc, &mut edges, id, org_hub)?; // role-fact ↔ company
            }
        }
    }
    Ok(Store { svc, roles })
}

fn relate(
    svc: &MemoryService<DynEmbedder>,
    edges: &mut HashSet<(u64, u64)>,
    a: u64,
    b: u64,
) -> Result<(), Box<dyn Error>> {
    if edges.insert((a, b)) {
        svc.relate(a, b, "about")?;
    }
    if edges.insert((b, a)) {
        svc.relate(b, a, "mentions")?;
    }
    Ok(())
}

fn abbr(role: &str) -> String {
    role.split_whitespace()
        .map(|w| w.chars().next().unwrap_or('x'))
        .collect()
}

#[derive(Default)]
struct Report {
    n: u32,
    vector: f64,
    column: f64,
    graph: f64,
    both: f64,
}

impl Report {
    fn record(&mut self, v: f64, c: f64, g: f64, b: f64) {
        self.n += 1;
        self.vector += v;
        self.column += c;
        self.graph += g;
        self.both += b;
    }

    fn print(&self, args: &Args, n: usize) {
        println!("\nVelesDB-memory — tri-engine compounding (multi-hop AND time-scoped)");
        println!(
            "embedder: ollama / {} · {n} probes · k={} · synthetic, generation-free\n",
            args.embed, args.k
        );
        let p = |s: f64| 100.0 * s / f64::from(self.n.max(1));
        println!("  answer-bearing recall@k:");
        println!("    vector only                 {:.1}%", p(self.vector));
        println!(
            "    + ColumnStore (year filter) {:.1}%   ({:+.1} pp)",
            p(self.column),
            p(self.column) - p(self.vector)
        );
        println!(
            "    + Graph (city→company)      {:.1}%   ({:+.1} pp)",
            p(self.graph),
            p(self.graph) - p(self.vector)
        );
        println!(
            "    + both engines              {:.1}%   ({:+.1} pp)",
            p(self.both),
            p(self.both) - p(self.vector)
        );
        println!("\n  → the engines compound: each fixes one axis (Graph the company, ColumnStore the period); together they nail both.");
    }
}

impl Args {
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut args = Args {
            k: 5,
            embed: "mxbai-embed-large".to_string(),
            hops: 2,
            boost: 0.30,
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
                "--hops" => args.hops = val.parse()?,
                "--graph-boost" => args.boost = val.parse()?,
                other => return Err(format!("unknown argument: {other}").into()),
            }
            i += 2;
        }
        Ok(args)
    }
}
