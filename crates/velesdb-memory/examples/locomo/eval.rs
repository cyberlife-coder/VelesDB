//! Per-question evaluation under one retrieval mode.
//!
//! The two modes share an equal fact budget `k`, so the only difference is
//! where the facts come from:
//! - **vector** (graph off): the top-`k` facts by embedding similarity.
//! - **fused** (graph on): all three engines. The vector pool — optionally
//!   constrained by a `ColumnStore` date window when the question names a year
//!   (`recall_where`) — and the facts `why()` reaches by graph traversal are
//!   re-ranked together by `normalised_vector_similarity + graph_boost·
//!   is_connected`, keeping the top `k`. Connected facts are promoted by
//!   relevance, not forced in, so a strong vector hit is never blindly evicted.
//!
//! Both feed the same generator and judge, so any score gap is the fusion's.

use std::collections::HashSet;
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use velesdb_memory::{ColumnFilter, ColumnOp};

use crate::dataset::Qa;
use crate::ingest::Store;
use crate::judge;
use crate::ollama_gen::Generator;

/// How many facts graph traversal injected into a context, and how many
/// graph-mode contexts received at least one. Lets the report state how *active*
/// the graph was: a zero here means any score delta cannot be the graph's.
static GRAPH_INJECTED: AtomicUsize = AtomicUsize::new(0);
static GRAPH_ACTIVE_CONTEXTS: AtomicUsize = AtomicUsize::new(0);

/// `(facts injected by traversal, graph-mode contexts that received ≥1)`.
pub fn graph_activity() -> (usize, usize) {
    (
        GRAPH_INJECTED.load(Ordering::Relaxed),
        GRAPH_ACTIVE_CONTEXTS.load(Ordering::Relaxed),
    )
}

/// Retrieval/scoring knobs, fixed for a whole run.
#[derive(Clone, Copy)]
pub struct EvalCfg {
    pub k: usize,
    pub graph_boost: f64,
    pub hops: usize,
}

/// A retrieved fact carried into the generation context. `score` is its vector
/// similarity (0.0 for facts found only by graph traversal).
#[derive(Clone)]
struct RetrievedFact {
    id: u64,
    text: String,
    dia_ids: Vec<String>,
    score: f64,
}

/// Outcome of one QA under one mode.
pub struct ModeResult {
    /// A retrieved fact's source overlapped the gold `evidence`.
    pub evidence_hit: bool,
    /// Counted correct (LLM judge, or abstention for adversarial items).
    pub correct: bool,
    /// Token-F1 vs gold; `None` for adversarial items.
    pub f1: Option<f64>,
}

/// Evaluate one QA end-to-end under the chosen mode.
pub fn evaluate(
    store: &Store,
    generator: &Generator,
    qa: &Qa,
    cfg: EvalCfg,
    graph_on: bool,
) -> Result<ModeResult, Box<dyn Error>> {
    let facts = retrieve(store, &qa.question, cfg, graph_on)?;
    let evidence_hit = facts
        .iter()
        .any(|f| f.dia_ids.iter().any(|id| qa.evidence.contains(id)));
    let texts: Vec<String> = facts.into_iter().map(|f| f.text).collect();
    let candidate = judge::answer(generator, &qa.question, &texts)?;
    let (correct, f1) = score(generator, qa, &candidate)?;
    Ok(ModeResult {
        evidence_hit,
        correct,
        f1,
    })
}

/// Grade a candidate answer: adversarial items want abstention; answerable
/// items use the LLM judge plus a logged F1.
fn score(
    generator: &Generator,
    qa: &Qa,
    candidate: &str,
) -> Result<(bool, Option<f64>), Box<dyn Error>> {
    if qa.category.is_adversarial() {
        return Ok((judge::abstained(candidate), None));
    }
    let Some(gold) = judge::gold_answer(qa) else {
        // Answerable category but no reference: cannot grade, count as missed.
        return Ok((false, Some(0.0)));
    };
    let correct = judge::judge_correct(generator, &qa.question, gold, candidate)?;
    Ok((correct, Some(judge::f1(candidate, gold))))
}

/// Vector-candidate pool depth: the semantic candidates both modes draw from,
/// deep enough that a relevant-but-not-top fact promoted by the graph is in it.
const POOL_FACTOR: usize = 8;
const POOL_MIN: usize = 64;

/// Retrieve the equal-budget fact set for `question` under the chosen mode.
///
/// Vector mode takes the top `k` of the vector pool. Graph mode **fuses**: it
/// re-ranks the union of the vector pool and the facts `why()` reaches by
/// traversal, scoring each by `normalised_vector_similarity + graph_boost` when
/// the fact is graph-connected, then takes the top `k`. A strong vector fact
/// keeps its place; a graph-connected fact only displaces it when the boost
/// lifts it higher — so multi-hop facts surface without evicting good vector
/// hits, the failure of plain slot-reservation.
fn retrieve(
    store: &Store,
    question: &str,
    cfg: EvalCfg,
    graph_on: bool,
) -> Result<Vec<RetrievedFact>, Box<dyn Error>> {
    if !graph_on {
        // Vector-only baseline: no ColumnStore predicate, no graph.
        let pool = vector_pool(store, question, pool_size(cfg.k), &[])?;
        return Ok(pool.into_iter().take(cfg.k).collect());
    }
    // Fused mode: a ColumnStore date window when the question is time-scoped,
    // plus graph traversal — all three engines.
    let filters = temporal_filters(question);
    let pool = vector_pool(store, question, pool_size(cfg.k), &filters)?;
    let reached = graph_reached(store, question, cfg.hops)?;
    Ok(fuse(pool, &reached, cfg))
}

/// A `ColumnStore` date window derived from a year named in the question (e.g.
/// "...in 2023"), so a time-scoped question is constrained to facts from that
/// year. Empty when the question names no year — most LoCoMo temporal questions
/// ask *for* a date rather than scoping *to* one, so this fires rarely.
fn temporal_filters(question: &str) -> Vec<ColumnFilter> {
    let Some(year) = find_year(question) else {
        return Vec::new();
    };
    let low = year * 10_000;
    vec![
        ColumnFilter {
            field: "ts".to_string(),
            op: ColumnOp::Ge,
            value: json!(low),
        },
        ColumnFilter {
            field: "ts".to_string(),
            op: ColumnOp::Le,
            value: json!(low + 9_999),
        },
    ]
}

/// The first plausible calendar year (1990-2099) named in `question`.
fn find_year(question: &str) -> Option<i64> {
    question
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|token| token.parse::<i64>().ok())
        .find(|year| (1990..=2099).contains(year))
}

/// Pool size: a deep candidate set, never below [`POOL_MIN`].
fn pool_size(k: usize) -> usize {
    k.saturating_mul(POOL_FACTOR).max(POOL_MIN)
}

/// The top-`want` fact memories by vector similarity, in score order, each
/// carrying its similarity (entity hubs filtered out, oversampling to survive
/// the filter).
fn vector_pool(
    store: &Store,
    question: &str,
    want: usize,
    filters: &[ColumnFilter],
) -> Result<Vec<RetrievedFact>, Box<dyn Error>> {
    let oversample = want.saturating_mul(2).saturating_add(16);
    let hits = if filters.is_empty() {
        store.svc.recall(question, oversample, None)?
    } else {
        store.svc.recall_where(question, oversample, filters)?
    };
    let mut out = Vec::with_capacity(want);
    for hit in hits {
        if store.is_fact(hit.id) {
            out.push(RetrievedFact {
                id: hit.id,
                text: hit.content,
                dia_ids: store.dia_ids(hit.id).to_vec(),
                score: f64::from(hit.score),
            });
            if out.len() == want {
                break;
            }
        }
    }
    Ok(out)
}

/// The facts `why()` reaches by traversal (hop ≥ 1) from the question's best
/// vector seed. Their text comes from the traversal itself, so a fact the vector
/// pool never ranked — the very thing pure similarity misses — can still fuse
/// in. Vector score is 0: their relevance is the graph boost alone.
fn graph_reached(
    store: &Store,
    question: &str,
    hops: usize,
) -> Result<Vec<RetrievedFact>, Box<dyn Error>> {
    let explanation = store.svc.why(question, hops, None)?;
    Ok(explanation
        .nodes
        .into_iter()
        .filter(|node| node.hop >= 1 && store.is_fact(node.id))
        .map(|node| RetrievedFact {
            id: node.id,
            dia_ids: store.dia_ids(node.id).to_vec(),
            text: node.content,
            score: 0.0,
        })
        .collect())
}

/// Fuse vector and graph: re-rank `pool ∪ reached` by
/// `normalised_vector_similarity + graph_boost·is_connected`, take the top `k`.
/// Equal budget, no blind eviction.
fn fuse(pool: Vec<RetrievedFact>, reached: &[RetrievedFact], cfg: EvalCfg) -> Vec<RetrievedFact> {
    let connected: HashSet<u64> = reached.iter().map(|f| f.id).collect();
    let vector_top: HashSet<u64> = pool.iter().take(cfg.k).map(|f| f.id).collect();
    let max_score = pool
        .iter()
        .map(|f| f.score)
        .fold(f64::MIN, f64::max)
        .max(f64::EPSILON);

    let mut candidates: Vec<RetrievedFact> = pool;
    let present: HashSet<u64> = candidates.iter().map(|f| f.id).collect();
    candidates.extend(reached.iter().filter(|f| !present.contains(&f.id)).cloned());

    candidates.sort_by(|a, b| {
        fused_score(b, &connected, max_score, cfg)
            .total_cmp(&fused_score(a, &connected, max_score, cfg))
    });
    candidates.truncate(cfg.k);
    record_injection(
        candidates
            .iter()
            .filter(|f| !vector_top.contains(&f.id))
            .count(),
    );
    candidates
}

/// `normalised_vector_similarity + graph_boost` when the fact is graph-connected.
fn fused_score(
    fact: &RetrievedFact,
    connected: &HashSet<u64>,
    max_score: f64,
    cfg: EvalCfg,
) -> f64 {
    let boost = if connected.contains(&fact.id) {
        cfg.graph_boost
    } else {
        0.0
    };
    fact.score / max_score + boost
}

/// Tally how many of the chosen facts the graph promoted in (i.e. that were not
/// already in the vector top-`k`). Zero ⇒ the graph changed nothing here.
fn record_injection(injected: usize) {
    if injected > 0 {
        GRAPH_INJECTED.fetch_add(injected, Ordering::Relaxed);
        GRAPH_ACTIVE_CONTEXTS.fetch_add(1, Ordering::Relaxed);
    }
}
