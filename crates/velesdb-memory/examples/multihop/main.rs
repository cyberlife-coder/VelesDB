//! `HotpotQA` supporting-fact retrieval benchmark for velesdb-memory.
//!
//! Generation-free: it scores whether the tri-engine surfaces the gold
//! *supporting-fact* sentences, not whether a model answers correctly. Each
//! question carries 10 paragraphs (2 gold + 8 distractors) split into sentences;
//! we ingest every sentence as a memory, build an entity graph from the
//! paragraph titles (each sentence links to its own title and to any other
//! paragraph-title it mentions — the multi-hop *bridge*), then compare
//! supporting-fact recall of pure **vector** recall vs the **fused**
//! vector+graph (`why()`) retrieval. The graph's whole job is to pull the
//! second-hop sentence into the budget; this measures exactly that.
//!
//! ```text
//! ollama pull mxbai-embed-large
//! cargo run --release -p velesdb-memory --features ollama --example multihop -- \
//!   --embed-model mxbai-embed-large --k 8 --questions 300
//! ```

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;

use serde::Deserialize;
use velesdb_memory::DynEmbedder;
use velesdb_memory::{MemoryService, OllamaEmbedder, DEFAULT_OLLAMA_URL};

#[derive(Deserialize)]
struct SupportingFacts {
    title: Vec<String>,
    sent_id: Vec<i64>,
}

#[derive(Deserialize)]
struct Context {
    title: Vec<String>,
    sentences: Vec<Vec<String>>,
}

// benchmark harness: field name mirrors dataset JSON key
#[allow(clippy::struct_field_names)]
#[derive(Deserialize)]
struct Question {
    question: String,
    #[serde(rename = "type")]
    qtype: String,
    supporting_facts: SupportingFacts,
    context: Context,
}

struct Args {
    dataset: PathBuf,
    k: usize,
    embed: String,
    n: usize,
    boost: f64,
    hops: usize,
    /// Weight a graph-bridge by the inverse-document-frequency of the connecting
    /// title shared with the question's vector neighbourhood (a rare title is a
    /// real bridge; a common one is noise), instead of a flat boost.
    idf: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let bytes = std::fs::read(&args.dataset).map_err(|e| {
        format!(
            "cannot read {} ({e}); fetch HotpotQA dev first",
            args.dataset.display()
        )
    })?;
    let data: Vec<Question> = serde_json::from_slice(&bytes)?;
    let take = args.n.min(data.len());
    let mut report = Report::default();
    for (pos, q) in data.iter().take(take).enumerate() {
        let gold = gold_set(q);
        if gold.is_empty() {
            continue;
        }
        let store = build_store(q, &args, pos)?;
        let vector = store.coverage(&store.retrieve(&q.question, &args, false)?, &gold);
        let fused = store.coverage(&store.retrieve(&q.question, &args, true)?, &gold);
        report.record(&q.qtype, vector, fused);
        drop(store);
        cleanup(pos);
        if (pos + 1) % 50 == 0 {
            eprintln!("  {}/{} questions", pos + 1, take);
        }
    }
    report.print(&args, take);
    Ok(())
}

/// The gold supporting facts as `(title, sentence_id)` pairs.
fn gold_set(q: &Question) -> HashSet<(String, i64)> {
    q.supporting_facts
        .title
        .iter()
        .cloned()
        .zip(q.supporting_facts.sent_id.iter().copied())
        .collect()
}

/// The ingested store for one question: the service plus a sentence-id index and
/// the entity-hub adjacency the graph traverses.
struct Store {
    svc: MemoryService<DynEmbedder>,
    /// sentence memory id → its `(title, sentence_id)` supporting-fact key.
    sent: HashMap<u64, (String, i64)>,
    /// sentence id → the title-entity hub ids it links to.
    sent_entities: HashMap<u64, Vec<u64>>,
    /// title-entity hub id → its degree (how many sentences link to it).
    entity_degree: HashMap<u64, usize>,
    /// number of sentences, for the idf normalisation.
    n_sent: f64,
}

impl Store {
    /// True when `id` is a sentence (not an entity hub).
    fn is_sentence(&self, id: u64) -> bool {
        self.sent.contains_key(&id)
    }

    /// Normalised inverse-document-frequency of a title-entity in `[0,1]`: ~1 when
    /// it links a single sentence (a precise bridge), → 0 as it links many.
    fn idf(&self, eid: u64) -> f64 {
        let degree = self.entity_degree.get(&eid).copied().unwrap_or(0);
        if degree == 0 || self.n_sent <= 1.0 {
            return 0.0;
        }
        let d = f64::from(u32::try_from(degree).unwrap_or(u32::MAX));
        (self.n_sent / d).ln() / self.n_sent.ln()
    }

    /// Hub ids of the top-`k` vector-hit sentences — the question's neighbourhood.
    fn seed_entities(&self, pool: &[(u64, f64)], k: usize) -> HashSet<u64> {
        pool.iter()
            .take(k)
            .flat_map(|(id, _)| self.sent_entities.get(id).into_iter().flatten().copied())
            .collect()
    }

    /// Bridge strength of a reached sentence: the max idf over the title-entities
    /// it shares with `seeds` (0 when it shares none).
    fn bridge_weight(&self, sentence: u64, seeds: &HashSet<u64>) -> f64 {
        self.sent_entities
            .get(&sentence)
            .into_iter()
            .flatten()
            .filter(|e| seeds.contains(e))
            .map(|e| self.idf(*e))
            .fold(0.0, f64::max)
    }

    /// Supporting-fact recall of `retrieved` against `gold`: fraction of gold
    /// `(title, sent_id)` pairs present, plus whether *all* were retrieved.
    fn coverage(&self, retrieved: &[u64], gold: &HashSet<(String, i64)>) -> Cov {
        let found: HashSet<&(String, i64)> = retrieved
            .iter()
            .filter_map(|id| self.sent.get(id))
            .filter(|key| gold.contains(*key))
            .collect();
        let hit = u32::try_from(found.len()).unwrap_or(u32::MAX);
        let total = u32::try_from(gold.len()).unwrap_or(u32::MAX);
        Cov {
            recall: f64::from(hit) / f64::from(total),
            complete: found.len() == gold.len(),
        }
    }

    /// The top-`k` sentence ids for `question`. Vector mode takes the vector top-k;
    /// fused mode re-ranks the vector pool unioned with the graph-reached sentences
    /// by `normalised_similarity + boost·is_graph_reached`.
    fn retrieve(
        &self,
        question: &str,
        args: &Args,
        graph_on: bool,
    ) -> Result<Vec<u64>, Box<dyn Error>> {
        let pool: Vec<(u64, f64)> = self
            .svc
            .recall(question, args.k * 8 + 16, None)?
            .into_iter()
            .filter(|r| self.is_sentence(r.id))
            .map(|r| (r.id, f64::from(r.score)))
            .collect();
        if !graph_on {
            return Ok(pool.into_iter().take(args.k).map(|(id, _)| id).collect());
        }
        let reached: HashSet<u64> = self
            .svc
            .why(question, args.hops, None)?
            .nodes
            .into_iter()
            .filter(|n| n.hop >= 1 && self.is_sentence(n.id))
            .map(|n| n.id)
            .collect();
        let max = pool
            .iter()
            .map(|(_, s)| *s)
            .fold(f64::MIN, f64::max)
            .max(f64::EPSILON);
        let mut cand: HashMap<u64, f64> = pool.iter().map(|(id, s)| (*id, s / max)).collect();
        let seeds = if args.idf {
            self.seed_entities(&pool, args.k)
        } else {
            HashSet::new()
        };
        for id in &reached {
            // idf mode: weight by the rarity of the shared bridge title, drop a
            // reached sentence that shares no seed title. Flat mode: fixed boost.
            let weight = if args.idf {
                self.bridge_weight(*id, &seeds)
            } else {
                1.0
            };
            if weight > 0.0 {
                *cand.entry(*id).or_insert(0.0) += args.boost * weight;
            }
        }
        let mut scored: Vec<(u64, f64)> = cand.into_iter().collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        Ok(scored.into_iter().take(args.k).map(|(id, _)| id).collect())
    }
}

/// Outcome of one question under one mode.
#[derive(Clone, Copy)]
struct Cov {
    recall: f64,
    complete: bool,
}

/// Ingest a question's 10 paragraphs as sentences + a title-entity graph.
fn build_store(q: &Question, args: &Args, pos: usize) -> Result<Store, Box<dyn Error>> {
    cleanup(pos);
    let embedder: DynEmbedder = Box::new(OllamaEmbedder::new(DEFAULT_OLLAMA_URL, &args.embed)?);
    let svc = MemoryService::open(store_dir(pos), embedder)?;
    let mut store = Store {
        svc,
        sent: HashMap::new(),
        sent_entities: HashMap::new(),
        entity_degree: HashMap::new(),
        n_sent: 0.0,
    };
    let titles: Vec<String> = q.context.title.clone();
    let titles_lc: Vec<String> = titles.iter().map(|t| t.to_lowercase()).collect();
    let mut hub: HashMap<String, u64> = HashMap::new();
    let mut edges: HashSet<(u64, u64)> = HashSet::new();

    for (title, sentences) in titles.iter().zip(q.context.sentences.iter()) {
        for (sid, text) in sentences.iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            let id = store.svc.remember(text, &[], None)?;
            // benchmark harness: sentence index fits i64
            #[allow(clippy::cast_possible_wrap)]
            store
                .sent
                .entry(id)
                .or_insert_with(|| (title.clone(), sid as i64));
            // Link the sentence to its own title-entity and to any other
            // paragraph-title it mentions — that mention is the multi-hop bridge.
            let lc = text.to_lowercase();
            let mut eids = Vec::new();
            for (other, other_lc) in titles.iter().zip(titles_lc.iter()) {
                if other == title || lc.contains(other_lc.as_str()) {
                    let eid = entity(&store.svc, &mut hub, other)?;
                    link(&store.svc, &mut edges, id, eid)?;
                    if !eids.contains(&eid) {
                        eids.push(eid);
                    }
                }
            }
            store.sent_entities.entry(id).or_insert(eids);
        }
    }
    // Title-entity degree = number of distinct sentences linking to it.
    let mut degree: HashMap<u64, usize> = HashMap::new();
    for eids in store.sent_entities.values() {
        for &eid in eids {
            *degree.entry(eid).or_insert(0) += 1;
        }
    }
    store.entity_degree = degree;
    store.n_sent = f64::from(u32::try_from(store.sent.len()).unwrap_or(u32::MAX));
    Ok(store)
}

/// Get or create the entity hub for a paragraph title.
fn entity(
    svc: &MemoryService<DynEmbedder>,
    hub: &mut HashMap<String, u64>,
    title: &str,
) -> Result<u64, Box<dyn Error>> {
    if let Some(&id) = hub.get(title) {
        return Ok(id);
    }
    let id = svc.remember(&format!("Entity: {title}"), &[], None)?;
    hub.insert(title.to_string(), id);
    Ok(id)
}

/// Relate sentence ↔ entity in both directions (`why()` follows outgoing edges).
fn link(
    svc: &MemoryService<DynEmbedder>,
    edges: &mut HashSet<(u64, u64)>,
    sentence: u64,
    entity: u64,
) -> Result<(), Box<dyn Error>> {
    if edges.insert((sentence, entity)) {
        svc.relate(sentence, entity, "about")?;
    }
    if edges.insert((entity, sentence)) {
        svc.relate(entity, sentence, "mentions")?;
    }
    Ok(())
}

fn store_dir(pos: usize) -> PathBuf {
    std::env::temp_dir().join(format!("velesdb-hotpot-{}-{pos}", std::process::id()))
}

fn cleanup(pos: usize) {
    let _ = std::fs::remove_dir_all(store_dir(pos));
}

/// Per-type supporting-fact recall tallies.
#[derive(Default)]
struct Report {
    cells: HashMap<String, Cell>,
}

#[derive(Default, Clone, Copy)]
struct Cell {
    n: u32,
    vector_recall: f64,
    fused_recall: f64,
    vector_complete: u32,
    fused_complete: u32,
}

impl Report {
    fn record(&mut self, qtype: &str, vector: Cov, fused: Cov) {
        let cell = self.cells.entry(qtype.to_string()).or_default();
        cell.n += 1;
        cell.vector_recall += vector.recall;
        cell.fused_recall += fused.recall;
        cell.vector_complete += u32::from(vector.complete);
        cell.fused_complete += u32::from(fused.complete);
    }

    fn print(&self, args: &Args, samples: usize) {
        println!("\nVelesDB-memory — HotpotQA supporting-fact retrieval (generation-free)");
        println!(
            "embedder: ollama / {} · {samples} questions · k={}, boost={}, {} hops\n",
            args.embed, args.k, args.boost, args.hops
        );
        println!(
            "  {:<12}{:>6}   {:>9} {:>9}   {:>9} {:>9}",
            "type", "n", "vec-rec", "fus-rec", "vec-cplt", "fus-cplt"
        );
        let mut total = Cell::default();
        let mut types: Vec<&String> = self.cells.keys().collect();
        types.sort();
        for t in types {
            let c = self.cells[t];
            self.row(t, c);
            total.n += c.n;
            total.vector_recall += c.vector_recall;
            total.fused_recall += c.fused_recall;
            total.vector_complete += c.vector_complete;
            total.fused_complete += c.fused_complete;
        }
        self.row("ALL", total);
        let vr = pct(total.vector_recall, total.n);
        let fr = pct(total.fused_recall, total.n);
        println!(
            "\n→ supporting-fact recall@k: vector {vr:.1}% → fused {fr:.1}% ({:+.1} pp)   ·   \
both-facts complete: vector {:.1}% → fused {:.1}%",
            fr - vr,
            100.0 * f64::from(total.vector_complete) / f64::from(total.n.max(1)),
            100.0 * f64::from(total.fused_complete) / f64::from(total.n.max(1)),
        );
        println!("  (recall = share of a question's gold supporting-fact sentences in the top-k)");
    }

    #[allow(clippy::unused_self)]
    fn row(&self, label: &str, c: Cell) {
        println!(
            "  {:<12}{:>6}   {:>8.1}% {:>8.1}%   {:>8.1}% {:>8.1}%",
            label,
            c.n,
            pct(c.vector_recall, c.n),
            pct(c.fused_recall, c.n),
            100.0 * f64::from(c.vector_complete) / f64::from(c.n.max(1)),
            100.0 * f64::from(c.fused_complete) / f64::from(c.n.max(1)),
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
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut args = Args {
            dataset: manifest.join("examples/multihop/data/hotpot_dev_300.json"),
            k: 8,
            embed: "mxbai-embed-large".to_string(),
            n: usize::MAX,
            boost: 0.15,
            hops: 2,
            idf: false,
        };
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < raw.len() {
            let flag = &raw[i];
            if flag == "--idf" {
                args.idf = true;
                i += 1;
                continue;
            }
            let val = raw
                .get(i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--dataset" => args.dataset = PathBuf::from(val),
                "--embed-model" => args.embed.clone_from(val),
                "--k" => args.k = val.parse()?,
                "--questions" => args.n = val.parse()?,
                "--graph-boost" => args.boost = val.parse()?,
                "--hops" => args.hops = val.parse()?,
                other => return Err(format!("unknown argument: {other}").into()),
            }
            i += 2;
        }
        Ok(args)
    }
}
