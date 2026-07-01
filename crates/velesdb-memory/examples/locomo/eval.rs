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

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use velesdb_memory::{ColumnFilter, ColumnOp};

use crate::dataset::{Category, Qa};
use crate::dump::{self, QuestionTrace};
use crate::ingest::Store;
use crate::judge;
use crate::ollama_gen::{Generator, TokenUsage};

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
// benchmark harness: independent ablation toggles, not a state machine
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy)]
pub struct EvalCfg {
    pub k: usize,
    pub graph_boost: f64,
    pub hops: usize,
    /// Apply graph fusion only to multi-hop questions (other categories take the
    /// pure vector path, where the graph was neutral-to-harmful).
    pub multihop_only: bool,
    /// Weight a graph link by the inverse-document-frequency of the connecting
    /// entity instead of a flat boost: a rare shared entity promotes its fact, a
    /// generic mega-hub barely does, and a fact sharing no entity is dropped.
    pub idf_weight: bool,
    /// How many top vector hits seed the graph reach. `1` (or less) uses the
    /// shipped single-seed `why()`; `>1` reaches siblings from the top-N vector
    /// facts directly — tests the audit's "single-seed is a bottleneck" finding.
    pub seed_breadth: usize,
    /// Date-stamp each fact in the generation context (sorted chronologically).
    /// The session date is retrieved but otherwise never reaches the answerer, so
    /// temporal questions are unanswerable despite high evidence recall.
    pub date_context: bool,
    /// Apply `date_context` only to temporally-framed questions. Dating every
    /// context lifts temporal hugely but distracts multi-hop reasoning (−10pp),
    /// so routing keeps the temporal win without the multi-hop regression.
    pub date_routed: bool,
    /// For temporally-framed questions, replace the terse answer prompt with a
    /// timeline + "now" anchor + step-by-step date-arithmetic scaffold ending in
    /// a `FINAL:` line. Targets duration/ordering questions the terse prompt and
    /// bare dates can't solve.
    pub temporal_scaffold: bool,
    /// General chain-of-thought answering: reason step by step over the retrieved
    /// facts before answering (ending in `FINAL:`). Targets the recall→accuracy
    /// gap (gold is retrieved but the terse answerer fails to reason over it),
    /// biggest on multi-hop. The temporal scaffold takes priority when both apply.
    pub cot: bool,
    /// Fuse a BM25 lexical ranking into the vector pool by RRF before taking the
    /// top-k. Targets the ranking miss the ceiling diagnostic exposed: multi-hop
    /// gold facts in the dense top-64 but below top-8.
    pub bm25: bool,
    /// Grade answers with Claude Opus 4.8 (via the `claude` CLI) instead of the
    /// local model — a stronger, vendor-neutral judge.
    pub claude_judge: bool,
    /// Generate the *answer* with Claude Opus 4.8 instead of the local model.
    /// Isolates the memory's ceiling from the generator's: our retrieval recall
    /// is ~84%, so a strong reasoner over the same retrieved facts shows how high
    /// accuracy goes when the generator is not the bottleneck (the memory layer
    /// is model-agnostic — plug in any LLM).
    pub claude_gen: bool,
}

/// A retrieved fact carried into the generation context. `score` is its vector
/// similarity (0.0 for facts found only by graph traversal); `graph_weight` is
/// its graph-link strength in `[0, 1]` (0.0 for facts not reached by the graph).
/// `pub(crate)` so `dump.rs` can serialise it for `--dump` without a second,
/// divergent copy of these fields.
#[derive(Clone)]
pub(crate) struct RetrievedFact {
    pub(crate) id: u64,
    pub(crate) text: String,
    pub(crate) dia_ids: Vec<String>,
    pub(crate) score: f64,
    pub(crate) graph_weight: f64,
    /// Session date (`YYYYMMDD`, 0 if unknown) for date-stamping the context.
    pub(crate) ts: i64,
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

/// Evaluate one QA end-to-end under the chosen mode. `trace`, when set, also
/// writes one JSONL record to the `--dump` sink for the `LoCoMo` research
/// analysis. Purely observational: it only reads values this function already
/// computes and never changes a prompt string, so the generation/judge cache
/// (keyed on `model + prompt`, see `ollama_gen.rs`) is identical whether
/// `--dump` is on or off.
pub fn evaluate(
    store: &Store,
    generator: &Generator,
    qa: &Qa,
    cfg: EvalCfg,
    graph_on: bool,
    trace: Option<QuestionTrace<'_>>,
) -> Result<ModeResult, Box<dyn Error>> {
    let (facts, raw) = retrieve(
        store,
        &qa.question,
        cfg,
        graph_on,
        qa.category,
        trace.is_some(),
    )?;
    let evidence_hit = any_evidence_hit(&facts, &qa.evidence);
    // Cloned (not moved) so `facts` survives to build the `--dump` record below
    // when a trace is requested — a handful of short strings, negligible next
    // to the network round-trip `judge::answer` makes right after.
    let dated: Vec<(i64, String)> = facts.iter().map(|f| (f.ts, f.text.clone())).collect();
    let temporal = temporal_flags(&qa.question, cfg);
    let now_ts = store.latest_ts();
    let (candidate, usage) = judge::answer(
        generator,
        &qa.question,
        &dated,
        temporal.date_on,
        now_ts,
        temporal.scaffold_on,
        cfg.cot,
        cfg.claude_gen,
    )?;
    let (correct, f1) = score(generator, qa, &candidate, cfg.claude_judge)?;
    let result = ModeResult {
        evidence_hit,
        correct,
        f1,
    };
    maybe_dump(
        trace,
        &record_inputs(
            qa, cfg, graph_on, &raw, &facts, &candidate, usage, &result, &temporal, now_ts,
        ),
    )?;
    Ok(result)
}

/// Build this question's `--dump` inputs from what `evaluate` has already
/// computed — kept out of `evaluate` so that function stays within budget.
#[allow(clippy::too_many_arguments)]
fn record_inputs<'a>(
    qa: &'a Qa,
    cfg: EvalCfg,
    graph_on: bool,
    raw: &'a [RetrievedFact],
    reranked: &'a [RetrievedFact],
    candidate: &'a str,
    usage: Option<TokenUsage>,
    verdict: &ModeResult,
    temporal: &TemporalFlags,
    latest_ts: i64,
) -> dump::RecordInputs<'a> {
    dump::RecordInputs {
        qa,
        cfg,
        graph_on,
        raw,
        reranked,
        candidate,
        usage,
        correct: verdict.correct,
        f1: verdict.f1,
        evidence_hit: verdict.evidence_hit,
        date_on: temporal.date_on,
        scaffold_on: temporal.scaffold_on,
        is_temporal_trigger: temporal.is_temporal_trigger,
        latest_ts,
    }
}

/// Does any retrieved fact's source overlap the gold `evidence`?
fn any_evidence_hit(facts: &[RetrievedFact], evidence: &[String]) -> bool {
    facts
        .iter()
        .any(|f| f.dia_ids.iter().any(|id| evidence.contains(id)))
}

/// Whether a question is temporally framed, and whether dating/the scaffold
/// prompt should fire for it — computed once, fed to both the answerer and
/// the `--dump` record.
struct TemporalFlags {
    is_temporal_trigger: bool,
    date_on: bool,
    scaffold_on: bool,
}

/// The scaffold only fires where dates apply (temporally-framed questions).
fn temporal_flags(question: &str, cfg: EvalCfg) -> TemporalFlags {
    let is_temporal_trigger = is_temporal_question(question);
    let date_on = cfg.date_context && (!cfg.date_routed || is_temporal_trigger);
    let scaffold_on = cfg.temporal_scaffold && date_on;
    TemporalFlags {
        is_temporal_trigger,
        date_on,
        scaffold_on,
    }
}

/// Write a `--dump` record when `trace` is set; a no-op otherwise. Isolated so
/// `evaluate`'s own complexity/length stays within the crate's budget.
fn maybe_dump(
    trace: Option<QuestionTrace<'_>>,
    inputs: &dump::RecordInputs<'_>,
) -> Result<(), Box<dyn Error>> {
    let Some(trace) = trace else {
        return Ok(());
    };
    dump::write_record(trace, inputs)
}

/// The source `dia_id` lists of the budgeted top-`k` facts retrieved for
/// `question` under the chosen mode — exactly the fact set the generator would
/// see. Exposed so a fast, LLM-free pass can score retrieval against gold
/// evidence without paying for generation or judging. Because it runs the real
/// budgeted [`retrieve`], a graph-injected distractor that evicts an evidence
/// fact is reflected here, unlike the unbudgeted explanation coverage.
pub fn retrieved_dia_ids(
    store: &Store,
    question: &str,
    cfg: EvalCfg,
    graph_on: bool,
    category: Category,
) -> Result<Vec<Vec<String>>, Box<dyn Error>> {
    Ok(retrieve(store, question, cfg, graph_on, category, false)?
        .0
        .into_iter()
        .map(|f| f.dia_ids)
        .collect())
}

/// Heuristic, label-free detector of a temporally-framed question — the kind
/// that needs dated context. Kept precise (strong cues only) so it fires on
/// "when / how long / what year / … ago" without flagging multi-hop questions,
/// whose reasoning the dated, chronologically-reordered context degrades.
fn is_temporal_question(question: &str) -> bool {
    const CUES: &[&str] = &[
        "when ",
        "what year",
        "which year",
        "what month",
        "which month",
        "what date",
        "what time",
        "how long",
        "how many days",
        "how many weeks",
        "how many months",
        "how many years",
        " ago",
        "how often",
        "how frequently",
    ];
    let q = question.to_lowercase();
    CUES.iter().any(|cue| q.contains(cue))
}

/// Grade a candidate answer: adversarial items want abstention; answerable
/// items use the LLM judge plus a logged F1.
fn score(
    generator: &Generator,
    qa: &Qa,
    candidate: &str,
    claude_judge: bool,
) -> Result<(bool, Option<f64>), Box<dyn Error>> {
    if qa.category.is_adversarial() {
        return Ok((judge::abstained(candidate), None));
    }
    let Some(gold) = judge::gold_answer(qa) else {
        // Answerable category but no reference: cannot grade, count as missed.
        return Ok((false, Some(0.0)));
    };
    let correct = judge::judge_correct(generator, &qa.question, gold, candidate, claude_judge)?;
    Ok((correct, Some(judge::f1(candidate, gold))))
}

/// Vector-candidate pool depth: the semantic candidates both modes draw from,
/// deep enough that a relevant-but-not-top fact promoted by the graph is in it.
const POOL_FACTOR: usize = 8;
const POOL_MIN: usize = 64;

/// Retrieve the equal-budget fact set for `question` under the chosen mode,
/// plus — when `want_raw` — the untruncated pre-budget candidates. The raw
/// list is read from the same pool/traversal this function already computes;
/// there is no second query and no change to the real (truncated) selection,
/// so passing `want_raw = false` (every call site except `--dump`) costs one
/// extra empty `Vec` and nothing else.
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
    category: Category,
    want_raw: bool,
) -> Result<(Vec<RetrievedFact>, Vec<RetrievedFact>), Box<dyn Error>> {
    // Route the graph to multi-hop questions only when asked: elsewhere it was
    // neutral-to-harmful, so the pure vector path is the safer default.
    let use_graph = graph_on && (!cfg.multihop_only || matches!(category, Category::MultiHop));
    if !use_graph {
        // Vector-only: no ColumnStore predicate, no graph. A date window still
        // applies on the fused path below for time-scoped questions.
        let pool = vector_pool(store, question, pool_size(cfg.k), &[])?;
        let raw = raw_if_wanted(&pool, &[], want_raw);
        if cfg.bm25 {
            return Ok((rrf_fuse(pool, store, question, cfg.k), raw));
        }
        return Ok((pool.into_iter().take(cfg.k).collect(), raw));
    }
    // Fused mode: a ColumnStore date window when the question is time-scoped,
    // plus graph traversal — all three engines.
    let filters = temporal_filters(question);
    let pool = vector_pool(store, question, pool_size(cfg.k), &filters)?;
    let reached = graph_reached(store, question, cfg)?;
    let raw = raw_if_wanted(&pool, &reached, want_raw);
    Ok((fuse(pool, &reached, cfg), raw))
}

/// `pool ∪ reached`, uncloned (empty) unless `--dump` actually wants it — kept
/// out of `retrieve` so its branching doesn't add to that function's CCN.
fn raw_if_wanted(
    pool: &[RetrievedFact],
    reached: &[RetrievedFact],
    want_raw: bool,
) -> Vec<RetrievedFact> {
    if !want_raw {
        return Vec::new();
    }
    let mut all = pool.to_vec();
    all.extend(reached.iter().cloned());
    all
}

/// Reciprocal Rank Fusion of the dense vector pool with a BM25 lexical ranking:
/// `score(d) = Σ 1/(RRF_K + rank_in_list)`, take the top-`k`. A fact the embedder
/// buried below the budget but BM25 ranks high (a literal name/number match) is
/// pulled in, and vice-versa — the standard fix for the ranking miss the ceiling
/// diagnostic exposed (dense recall@8 ≪ recall@64).
#[allow(clippy::similar_names)]
fn rrf_fuse(
    pool: Vec<RetrievedFact>,
    store: &Store,
    question: &str,
    k: usize,
) -> Vec<RetrievedFact> {
    const RRF_K: f64 = 60.0;
    const BM25_DEPTH: usize = 64;
    let dense_rank: HashMap<u64, usize> = pool.iter().enumerate().map(|(i, f)| (f.id, i)).collect();
    let bm25_rank: HashMap<u64, usize> = store
        .bm25_search(question)
        .into_iter()
        .take(BM25_DEPTH)
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect();
    let mut ids: HashSet<u64> = dense_rank.keys().copied().collect();
    ids.extend(bm25_rank.keys().copied());
    let mut scored: Vec<(u64, f64)> = ids
        .into_iter()
        .map(|id| {
            let mut score = 0.0;
            if let Some(&r) = dense_rank.get(&id) {
                score += 1.0 / (RRF_K + rank_f(r));
            }
            if let Some(&r) = bm25_rank.get(&id) {
                score += 1.0 / (RRF_K + rank_f(r));
            }
            (id, score)
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(k);
    let mut by_id: HashMap<u64, RetrievedFact> = pool.into_iter().map(|f| (f.id, f)).collect();
    scored
        .into_iter()
        .map(|(id, _)| {
            by_id.remove(&id).unwrap_or_else(|| RetrievedFact {
                id,
                text: store.fact_text(id).to_string(),
                dia_ids: store.dia_ids(id).to_vec(),
                score: 0.0,
                graph_weight: 0.0,
                ts: store.fact_ts(id),
            })
        })
        .collect()
}

/// A rank index as `f64` for RRF scoring (ranks are small, the clamp never bites).
fn rank_f(rank: usize) -> f64 {
    f64::from(u32::try_from(rank).unwrap_or(u32::MAX))
}

/// A `ColumnStore` date window derived from a year named in the question (e.g.
/// "...in 2023"), so a time-scoped question is constrained to facts from that
/// year. Empty when the question names no year — most `LoCoMo` temporal questions
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
                graph_weight: 0.0,
                ts: store.fact_ts(hit.id),
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
/// in. Vector score is 0: their relevance rides on `graph_weight`.
///
/// With `idf_weight` on, that weight is the rarity of the most *specific* entity
/// the reached fact shares with the question's vector neighbourhood — so a fact
/// linked through a rare, on-topic entity outranks one dangling off a generic
/// mega-hub, and a fact sharing no entity at all (pure topical drift) is dropped.
/// With it off, every reached fact carries weight 1.0 (the original flat boost).
fn graph_reached(
    store: &Store,
    question: &str,
    cfg: EvalCfg,
) -> Result<Vec<RetrievedFact>, Box<dyn Error>> {
    if cfg.seed_breadth > 1 {
        return graph_reached_multiseed(store, question, cfg);
    }
    let explanation = store.svc.why(question, cfg.hops, None)?;
    let seed_entities = if cfg.idf_weight {
        seed_entity_set(store, question, cfg.k)?
    } else {
        HashSet::new()
    };
    Ok(explanation
        .nodes
        .into_iter()
        .filter(|node| node.hop >= 1 && store.is_fact(node.id))
        .map(|node| {
            let weight = if cfg.idf_weight {
                connection_weight(store, node.id, &seed_entities)
            } else {
                1.0
            };
            RetrievedFact {
                id: node.id,
                dia_ids: store.dia_ids(node.id).to_vec(),
                text: node.content,
                score: 0.0,
                graph_weight: weight,
                ts: store.fact_ts(node.id),
            }
        })
        // When idf-weighting, a fact sharing no entity with the seed neighbourhood
        // scores 0 — drop it rather than let a topical distractor compete.
        .filter(|fact| fact.graph_weight > 0.0)
        .collect())
}

/// Multi-seed graph reach: instead of the shipped single-seed `why()`, anchor on
/// the top-`seed_breadth` vector facts, hop through each of their entities to the
/// sibling facts, and weight every reached fact by the strongest connecting
/// entity (entity-IDF when on, else a flat 1.0). Directly tests the audit's
/// finding that one seed is a single point of failure — more anchors, more of
/// the multi-hop neighbourhood reached.
fn graph_reached_multiseed(
    store: &Store,
    question: &str,
    cfg: EvalCfg,
) -> Result<Vec<RetrievedFact>, Box<dyn Error>> {
    let seeds = top_vector_fact_ids(store, question, cfg.seed_breadth)?;
    let seed_set: HashSet<u64> = seeds.iter().copied().collect();
    let mut weights: HashMap<u64, f64> = HashMap::new();
    for &sid in &seeds {
        for &eid in store.fact_entity_ids(sid) {
            let w = if cfg.idf_weight {
                store.entity_idf(eid)
            } else {
                1.0
            };
            if w <= 0.0 {
                continue;
            }
            for &fid in store.entity_fact_ids(eid) {
                if seed_set.contains(&fid) {
                    continue;
                }
                let slot = weights.entry(fid).or_insert(0.0);
                *slot = slot.max(w);
            }
        }
    }
    Ok(weights
        .into_iter()
        .map(|(id, weight)| RetrievedFact {
            id,
            dia_ids: store.dia_ids(id).to_vec(),
            text: store.fact_text(id).to_string(),
            score: 0.0,
            graph_weight: weight,
            ts: store.fact_ts(id),
        })
        .collect())
}

/// The top-`n` fact ids by vector similarity (entity hubs skipped).
fn top_vector_fact_ids(
    store: &Store,
    question: &str,
    n: usize,
) -> Result<Vec<u64>, Box<dyn Error>> {
    let hits = store
        .svc
        .recall(question, n.saturating_mul(3).saturating_add(8), None)?;
    let mut ids = Vec::with_capacity(n);
    for hit in hits {
        if !store.is_fact(hit.id) {
            continue;
        }
        ids.push(hit.id);
        if ids.len() == n {
            break;
        }
    }
    Ok(ids)
}

/// The entity hubs of the question's top vector-hit facts — its semantic
/// neighbourhood. A graph link back through one of these is on-topic; a link
/// through an entity outside it is drift.
fn seed_entity_set(
    store: &Store,
    question: &str,
    k: usize,
) -> Result<HashSet<u64>, Box<dyn Error>> {
    let mut entities = HashSet::new();
    for fid in top_vector_fact_ids(store, question, k)? {
        entities.extend(store.fact_entity_ids(fid).iter().copied());
    }
    Ok(entities)
}

/// The strength of a reached fact's best link into the seed neighbourhood: the
/// maximum entity-IDF over the entities it shares with `seed_entities` (0 when
/// it shares none).
fn connection_weight(store: &Store, fact_id: u64, seed_entities: &HashSet<u64>) -> f64 {
    store
        .fact_entity_ids(fact_id)
        .iter()
        .filter(|eid| seed_entities.contains(eid))
        .map(|eid| store.entity_idf(*eid))
        .fold(0.0, f64::max)
}

/// Fuse vector and graph: re-rank `pool ∪ reached` by
/// `normalised_vector_similarity + graph_boost·graph_weight`, take the top `k`.
/// Equal budget, no blind eviction; the per-fact `graph_weight` (entity-IDF, or
/// a flat 1.0 when idf-weighting is off) decides how hard the graph promotes.
fn fuse(pool: Vec<RetrievedFact>, reached: &[RetrievedFact], cfg: EvalCfg) -> Vec<RetrievedFact> {
    let weights: HashMap<u64, f64> = reached.iter().map(|f| (f.id, f.graph_weight)).collect();
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
        fused_score(b, &weights, max_score, cfg)
            .total_cmp(&fused_score(a, &weights, max_score, cfg))
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

/// `normalised_vector_similarity + graph_boost·graph_weight`, the weight being
/// the reached fact's entity-IDF link strength (0 for facts the graph never
/// reached, so a pure vector hit keeps its bare similarity score).
fn fused_score(
    fact: &RetrievedFact,
    weights: &HashMap<u64, f64>,
    max_score: f64,
    cfg: EvalCfg,
) -> f64 {
    let weight = weights.get(&fact.id).copied().unwrap_or(0.0);
    fact.score / max_score + cfg.graph_boost * weight
}

/// Tally how many of the chosen facts the graph promoted in (i.e. that were not
/// already in the vector top-`k`). Zero ⇒ the graph changed nothing here.
fn record_injection(injected: usize) {
    if injected > 0 {
        GRAPH_INJECTED.fetch_add(injected, Ordering::Relaxed);
        GRAPH_ACTIVE_CONTEXTS.fetch_add(1, Ordering::Relaxed);
    }
}
