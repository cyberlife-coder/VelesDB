//! Real `LoCoMo` benchmark for velesdb-memory: does the graph improve answer
//! recall over pure vector search, apples-to-apples (same embeddings, same
//! fact budget, same judge)?
//!
//! ```text
//! # one-time: fetch the dataset + pull the local models
//! examples/locomo/fetch_dataset.sh
//! ollama pull all-minilm && ollama pull qwen3.6:35b-mlx
//!
//! # smoke (1 conversation), then the full run (all 10):
//! cargo run --release -p velesdb-memory --features ollama --example locomo -- --conversations 1
//! cargo run --release -p velesdb-memory --features ollama --example locomo
//! # LLM-free explanation benchmark (does the graph connect scattered evidence?):
//! cargo run --release -p velesdb-memory --features ollama --example locomo -- --explanation
//! ```
//!
//! Pipeline: extract facts from each session with a local LLM (tagged with the
//! gold `dia_id`s and session timestamp), ingest them as a fact↔entity graph,
//! then answer every QA twice — vector-only vs a tri-engine fusion (vector +
//! graph traversal + `ColumnStore` date window) — and score with a hybrid LLM
//! judge plus deterministic evidence-overlap and token-F1. `--explanation` runs
//! a separate, generator-free measure of the graph's evidence-connecting value.
//! Each conversation is benchmarked in isolation; the score reflects the
//! extractor too.

mod bm25;
mod dataset;
mod diagnose;
mod eval;
mod explain;
mod extract;
mod ingest;
mod judge;
mod ollama_gen;
mod parse;
mod report;
mod retrieval;

use std::error::Error;
use std::path::PathBuf;

use velesdb_memory::mcp::DynEmbedder;
use velesdb_memory::{MemoryService, OllamaEmbedder, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL};

use dataset::{Category, Sample};
use diagnose::DiagnoseReport;
use eval::EvalCfg;
use explain::ExplainReport;
use ingest::Store;
use ollama_gen::Generator;
use report::Report;
use retrieval::RetrievalReport;

/// Parsed command-line configuration.
struct Args {
    dataset: PathBuf,
    conversations: usize,
    max_qa: usize,
    model: String,
    /// Run the LLM-free explanation benchmark instead of the QA eval.
    explanation: bool,
    /// Run the LLM-free budgeted-recall benchmark instead of the QA eval.
    retrieval: bool,
    /// Run the LLM-free extraction-vs-retrieval coverage diagnosis.
    diagnose: bool,
    /// Extraction prompt version: 1 (topics) or 2 (specific referents).
    extract_version: u8,
    /// Restrict the QA eval to one category (cheap, targeted A/B runs).
    only_category: Option<Category>,
    /// Ollama embedding model (swappable to test stronger encoders).
    embed_model: String,
    cfg: EvalCfg,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse()?;
    let samples = dataset::load(&args.dataset)?;
    let take = args.conversations.min(samples.len());
    let cache = manifest_dir().join("examples/locomo/cache");
    let generator = Generator::new(&args.model, cache)?;
    if args.diagnose {
        run_diagnose(&generator, &samples, take, &args)
    } else if args.retrieval {
        run_retrieval(&generator, &samples, take, &args)
    } else if args.explanation {
        run_explanation(&generator, &samples, take, &args)
    } else {
        run_eval(&generator, &samples, take, &args)
    }
}

/// The LLM-free retrieval benchmark: does fusion improve budgeted evidence
/// recall@k over pure vector recall? Runs the real budgeted retrieval, so it
/// captures distractor eviction the explanation coverage cannot.
fn run_retrieval(
    generator: &Generator,
    samples: &[Sample],
    take: usize,
    args: &Args,
) -> Result<(), Box<dyn Error>> {
    let mut report = RetrievalReport::default();
    for (position, sample) in samples.iter().take(take).enumerate() {
        let (store, _facts) = prepare(
            generator,
            sample,
            position,
            take,
            args.extract_version,
            &args.embed_model,
        )?;
        for qa in sample
            .qa
            .iter()
            .take(qa_limit(args.max_qa, sample.qa.len()))
        {
            report.record(&store, qa, args.cfg)?;
        }
        finish(store, position);
    }
    report.print(args.cfg, take);
    Ok(())
}

/// The LLM-free coverage diagnosis: splits the evidence gap into extraction loss
/// vs retrieval loss, per category.
fn run_diagnose(
    generator: &Generator,
    samples: &[Sample],
    take: usize,
    args: &Args,
) -> Result<(), Box<dyn Error>> {
    let mut report = DiagnoseReport::default();
    for (position, sample) in samples.iter().take(take).enumerate() {
        let (store, _facts) = prepare(
            generator,
            sample,
            position,
            take,
            args.extract_version,
            &args.embed_model,
        )?;
        for qa in sample
            .qa
            .iter()
            .take(qa_limit(args.max_qa, sample.qa.len()))
        {
            report.record(&store, qa, args.cfg)?;
        }
        finish(store, position);
    }
    report.print(args.cfg, take);
    Ok(())
}

/// The QA benchmark: vector vs fused retrieval, answered and judged per category.
fn run_eval(
    generator: &Generator,
    samples: &[Sample],
    take: usize,
    args: &Args,
) -> Result<(), Box<dyn Error>> {
    let mut report = Report::default();
    let mut total_facts = 0usize;
    for (position, sample) in samples.iter().take(take).enumerate() {
        let (store, facts) = prepare(
            generator,
            sample,
            position,
            take,
            args.extract_version,
            &args.embed_model,
        )?;
        let qa_take = qa_limit(args.max_qa, sample.qa.len());
        for (done, qa) in sample.qa.iter().take(qa_take).enumerate() {
            if args.only_category.is_some_and(|c| c != qa.category) {
                continue;
            }
            let off = eval::evaluate(&store, generator, qa, args.cfg, false)?;
            let on = eval::evaluate(&store, generator, qa, args.cfg, true)?;
            report.record(qa.category, false, &off);
            report.record(qa.category, true, &on);
            if (done + 1) % 20 == 0 {
                eprintln!("        {}/{} QA evaluated", done + 1, qa_take);
            }
        }
        finish(store, position);
        total_facts += facts;
    }
    report.print(args.cfg, generator.model(), take, total_facts);
    let (injected, contexts) = eval::graph_activity();
    println!(
        "graph activity: traversal injected {injected} fact(s) across {contexts} graph-mode context(s)"
    );
    Ok(())
}

/// The LLM-free explanation benchmark: does the graph connect scattered evidence?
fn run_explanation(
    generator: &Generator,
    samples: &[Sample],
    take: usize,
    args: &Args,
) -> Result<(), Box<dyn Error>> {
    let mut report = ExplainReport::default();
    for (position, sample) in samples.iter().take(take).enumerate() {
        let (store, _facts) = prepare(
            generator,
            sample,
            position,
            take,
            args.extract_version,
            &args.embed_model,
        )?;
        for qa in sample
            .qa
            .iter()
            .take(qa_limit(args.max_qa, sample.qa.len()))
        {
            report.record(&store, qa, args.cfg)?;
        }
        finish(store, position);
    }
    report.print(args.cfg, take);
    Ok(())
}

/// Extract and ingest one conversation; returns its store and fact count.
fn prepare(
    generator: &Generator,
    sample: &Sample,
    position: usize,
    total: usize,
    extract_version: u8,
    embed_model: &str,
) -> Result<(Store, usize), Box<dyn Error>> {
    eprintln!(
        "[{}/{}] sample {} — {} sessions, {} turns, {} QA: extracting…",
        position + 1,
        total,
        sample.sample_id,
        sample.sessions.len(),
        sample.turn_count(),
        sample.qa.len(),
    );
    let facts = extract::extract_sample(generator, sample, extract_version)?;
    let store = ingest::build(open_service(position, embed_model)?, &facts)?;
    eprintln!("        {} facts ingested", facts.len());
    Ok((store, facts.len()))
}

/// Drop a conversation's store and wipe its directory.
fn finish(store: Store, position: usize) {
    drop(store);
    cleanup(position);
}

/// Open a fresh, isolated service for conversation `position`, embedding with
/// `embed_model`. The store dir is wiped first so a crashed prior run can never
/// leak stale facts into the run.
fn open_service(
    position: usize,
    embed_model: &str,
) -> Result<MemoryService<DynEmbedder>, Box<dyn Error>> {
    cleanup(position);
    let embedder: DynEmbedder = Box::new(OllamaEmbedder::new(DEFAULT_OLLAMA_URL, embed_model)?);
    Ok(MemoryService::open(store_dir(position), embedder)?)
}

/// Per-conversation on-disk store path (wiped before and after use).
fn store_dir(position: usize) -> PathBuf {
    std::env::temp_dir().join(format!("velesdb-locomo-{}-{position}", std::process::id()))
}

/// Remove a conversation's store directory, ignoring absence.
fn cleanup(position: usize) {
    let _ = std::fs::remove_dir_all(store_dir(position));
}

/// `max_qa == 0` means "all"; otherwise cap.
fn qa_limit(max_qa: usize, available: usize) -> usize {
    if max_qa == 0 {
        available
    } else {
        max_qa.min(available)
    }
}

/// The crate manifest directory, for resolving the default dataset and cache.
fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

impl Default for Args {
    fn default() -> Self {
        Self {
            dataset: manifest_dir().join("examples/locomo/data/locomo10.json"),
            conversations: 10,
            max_qa: 0,
            model: String::new(),
            explanation: false,
            retrieval: false,
            diagnose: false,
            extract_version: 1,
            only_category: None,
            embed_model: DEFAULT_OLLAMA_MODEL.to_string(),
            cfg: EvalCfg {
                // Gentle: the graph nudges, rarely evicting a strong vector hit.
                // On LoCoMo the graph is ~neutral; higher boosts only hurt more.
                k: 8,
                graph_boost: 0.15,
                hops: 2,
                multihop_only: false,
                idf_weight: false,
                seed_breadth: 1,
                date_context: false,
                date_routed: false,
                temporal_scaffold: false,
                cot: false,
                bm25: false,
                claude_judge: false,
                claude_gen: false,
            },
        }
    }
}

impl Args {
    /// Parse `--flag value` arguments over the defaults.
    fn parse() -> Result<Self, Box<dyn Error>> {
        let mut args = Args::default();
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;
        while i < raw.len() {
            let flag = &raw[i];
            if flag == "--explanation" {
                args.explanation = true;
                i += 1;
                continue;
            }
            if flag == "--retrieval" {
                args.retrieval = true;
                i += 1;
                continue;
            }
            if flag == "--diagnose" {
                args.diagnose = true;
                i += 1;
                continue;
            }
            if flag == "--multihop-only" {
                args.cfg.multihop_only = true;
                i += 1;
                continue;
            }
            if flag == "--idf-weight" {
                args.cfg.idf_weight = true;
                i += 1;
                continue;
            }
            if flag == "--extract-v2" {
                args.extract_version = 2;
                i += 1;
                continue;
            }
            if flag == "--date-context" {
                args.cfg.date_context = true;
                i += 1;
                continue;
            }
            if flag == "--date-route" {
                args.cfg.date_routed = true;
                i += 1;
                continue;
            }
            if flag == "--temporal-scaffold" {
                args.cfg.temporal_scaffold = true;
                i += 1;
                continue;
            }
            if flag == "--cot" {
                args.cfg.cot = true;
                i += 1;
                continue;
            }
            if flag == "--bm25" {
                args.cfg.bm25 = true;
                i += 1;
                continue;
            }
            if flag == "--claude-judge" {
                args.cfg.claude_judge = true;
                i += 1;
                continue;
            }
            if flag == "--claude-gen" {
                args.cfg.claude_gen = true;
                i += 1;
                continue;
            }
            let value = raw
                .get(i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?;
            args.set(flag, value)?;
            i += 2;
        }
        if args.cfg.k == 0 {
            return Err("--k must be at least 1".into());
        }
        Ok(args)
    }

    /// Apply one `--flag value` pair (string/float flags here, ints delegated).
    fn set(&mut self, flag: &str, value: &str) -> Result<(), Box<dyn Error>> {
        match flag {
            "--model" => self.model = value.to_string(),
            "--embed-model" => self.embed_model = value.to_string(),
            "--dataset" => self.dataset = PathBuf::from(value),
            "--only" => {
                self.only_category = Some(
                    Category::from_label(value)
                        .ok_or_else(|| format!("unknown category: {value}"))?,
                );
            }
            "--graph-boost" => self.cfg.graph_boost = value.parse()?,
            _ => self.set_numeric(flag, value)?,
        }
        Ok(())
    }

    /// Apply an integer `--flag value` pair.
    fn set_numeric(&mut self, flag: &str, value: &str) -> Result<(), Box<dyn Error>> {
        let n: usize = value.parse()?;
        match flag {
            "--conversations" => self.conversations = n,
            "--max-qa" => self.max_qa = n,
            "--k" => self.cfg.k = n,
            "--hops" => self.cfg.hops = n,
            "--seed-breadth" => self.cfg.seed_breadth = n,
            other => return Err(format!("unknown argument: {other}").into()),
        }
        Ok(())
    }
}
