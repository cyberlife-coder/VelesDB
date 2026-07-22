//! Per-question JSONL instrumentation for the `LoCoMo` temporal-decomposition
//! research analysis (`--dump <path>`).
//!
//! Purely observational: every field is read from data `eval.rs` already
//! computes for the real vector/graph/generation path. There is no second
//! retrieval call and no change to any prompt string, so the generation/judge
//! cache (content-addressed on `model + prompt`, see `ollama_gen.rs`) is
//! identical whether `--dump` is on or off.

use std::error::Error;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::Path;

use serde::Serialize;

use crate::dataset::Qa;
use crate::eval::{EvalCfg, RetrievedFact};
use crate::ollama_gen::TokenUsage;

/// One retrieved fact, as it appeared in either the pre-budget candidate pool
/// (`raw_facts`) or the final budgeted set the generator actually saw
/// (`reranked_facts`). Comparing the two on `evidence_hit` tells "never
/// retrieved" (missing from both) apart from "retrieved but evicted by
/// ranking" (present in `raw_facts` only).
#[derive(Serialize)]
pub struct FactRecord {
    pub rank: usize,
    pub id: u64,
    pub ts: i64,
    pub vector_score: f64,
    pub graph_weight: f64,
    pub evidence_hit: bool,
    pub dia_ids: Vec<String>,
    pub text: String,
}

/// One question's full evaluation trace under one mode (vector-only or fused).
// research JSONL row: independent observed fields, not a state machine
#[allow(clippy::struct_excessive_bools)]
#[derive(Serialize)]
pub struct QuestionRecord {
    pub conversation_id: String,
    pub question_idx: usize,
    pub graph_on: bool,
    pub category: String,
    pub question: String,
    pub gold: Option<String>,
    pub predicted: String,
    pub correct: bool,
    pub f1: Option<f64>,
    pub evidence_hit: bool,
    pub date_on: bool,
    pub scaffold_on: bool,
    pub prompt_kind: String,
    pub is_temporal_trigger: bool,
    pub latest_ts: i64,
    /// From Ollama's own reply, when the answer call actually reached the
    /// model (`None` on a cache hit or a Claude-CLI generation).
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// Dependency-free whitespace-word estimate over `reranked_facts` text —
    /// always available, a fallback/cross-check for `prompt_tokens`.
    pub context_tokens: usize,
    pub n_distinct_dates: usize,
    pub date_span_days: i64,
    pub raw_facts: Vec<FactRecord>,
    pub reranked_facts: Vec<FactRecord>,
}

/// A `--dump` sink: each run starts from a fresh file. A restarted run
/// overwrites rather than appending duplicate rows — the generation/judge
/// cache still resumes for free underneath, so a restart just re-runs to
/// completion instead of silently corrupting the analysis with dupes.
pub struct DumpSink {
    writer: BufWriter<File>,
}

impl DumpSink {
    pub fn create(path: &Path) -> Result<Self, Box<dyn Error>> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            writer: BufWriter::new(File::create(path)?),
        })
    }

    fn write(&mut self, record: &QuestionRecord) -> Result<(), Box<dyn Error>> {
        serde_json::to_writer(&mut self.writer, record)?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }
}

/// Bundles the `--dump` sink with the (conversation, question) coordinates
/// `eval::evaluate` doesn't otherwise need — the only new plumbing this
/// instrumentation requires (`Sample::sample_id` already existed).
pub struct QuestionTrace<'a> {
    pub conversation_id: &'a str,
    pub question_idx: usize,
    pub sink: &'a mut DumpSink,
}

/// Everything `eval::evaluate` has already computed by the time it's ready to
/// write a record — grouped so the call site isn't a many-argument function.
// research JSONL row inputs: independent observed fields, not a state machine
#[allow(clippy::struct_excessive_bools)]
pub struct RecordInputs<'a> {
    pub qa: &'a Qa,
    pub cfg: EvalCfg,
    pub graph_on: bool,
    pub raw: &'a [RetrievedFact],
    pub reranked: &'a [RetrievedFact],
    pub candidate: &'a str,
    pub usage: Option<TokenUsage>,
    pub correct: bool,
    pub f1: Option<f64>,
    pub evidence_hit: bool,
    pub date_on: bool,
    pub scaffold_on: bool,
    pub is_temporal_trigger: bool,
    pub latest_ts: i64,
}

/// Build and write one [`QuestionRecord`] — the only place that touches both
/// `RetrievedFact` (eval.rs's retrieval type) and the JSONL shape.
///
/// `trace` must be taken by value: it carries the sink's unique `&mut`
/// borrow, which a further `&QuestionTrace` could not re-lend.
#[allow(clippy::needless_pass_by_value)]
pub fn write_record(
    trace: QuestionTrace<'_>,
    inputs: &RecordInputs<'_>,
) -> Result<(), Box<dyn Error>> {
    let raw_facts = to_fact_records(inputs.raw, &inputs.qa.evidence);
    let reranked_facts = to_fact_records(inputs.reranked, &inputs.qa.evidence);
    let (n_distinct_dates, date_span_days) = date_span(&reranked_facts);
    let context_tokens = reranked_facts
        .iter()
        .map(|f| estimate_tokens(&f.text))
        .sum();
    let prompt_kind = if inputs.scaffold_on {
        "scaffold"
    } else if inputs.cfg.cot {
        "cot"
    } else {
        "terse"
    };
    let record = QuestionRecord {
        conversation_id: trace.conversation_id.to_string(),
        question_idx: trace.question_idx,
        graph_on: inputs.graph_on,
        category: inputs.qa.category.label().to_string(),
        question: inputs.qa.question.clone(),
        gold: inputs.qa.answer.clone(),
        predicted: inputs.candidate.to_string(),
        correct: inputs.correct,
        f1: inputs.f1,
        evidence_hit: inputs.evidence_hit,
        date_on: inputs.date_on,
        scaffold_on: inputs.scaffold_on,
        prompt_kind: prompt_kind.to_string(),
        is_temporal_trigger: inputs.is_temporal_trigger,
        latest_ts: inputs.latest_ts,
        prompt_tokens: inputs.usage.map(|u| u.prompt),
        completion_tokens: inputs.usage.map(|u| u.completion),
        context_tokens,
        n_distinct_dates,
        date_span_days,
        raw_facts,
        reranked_facts,
    };
    trace.sink.write(&record)
}

/// Rank + evidence-hit each fact against the gold `evidence` `dia_ids`.
fn to_fact_records(facts: &[RetrievedFact], evidence: &[String]) -> Vec<FactRecord> {
    facts
        .iter()
        .enumerate()
        .map(|(rank, f)| FactRecord {
            rank,
            id: f.id,
            ts: f.ts,
            vector_score: f.score,
            graph_weight: f.graph_weight,
            evidence_hit: f.dia_ids.iter().any(|id| evidence.contains(id)),
            dia_ids: f.dia_ids.clone(),
            text: f.text.clone(),
        })
        .collect()
}

/// A dependency-free token-count estimate (whitespace-separated words). Not a
/// real tokenizer; a cheap, always-available proxy for when Ollama's own
/// `prompt_tokens`/`completion_tokens` are absent (cache hit, Claude-gen path).
fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace().count()
}

/// `(distinct known dates, day-span)` over a fact list's `ts` (`YYYYMMDD`,
/// `0` = unknown, excluded from both).
fn date_span(facts: &[FactRecord]) -> (usize, i64) {
    let mut days: Vec<i64> = facts
        .iter()
        .map(|f| f.ts)
        .filter(|&ts| ts > 0)
        .filter_map(ymd_to_ordinal)
        .collect();
    days.sort_unstable();
    days.dedup();
    let span = match (days.first(), days.last()) {
        (Some(&lo), Some(&hi)) => hi - lo,
        _ => 0,
    };
    (days.len(), span)
}

/// Days since a fixed epoch for a `YYYYMMDD` key (Howard Hinnant's
/// `days_from_civil`, proleptic Gregorian) — `None` for an implausible date.
fn ymd_to_ordinal(ts: i64) -> Option<i64> {
    let (year, month, day) = crate::judge::decompose_ymd(ts)?;
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let month_shifted = (month + 9) % 12;
    let day_of_year = (153 * month_shifted + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + day_of_year;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
#[path = "dump/dump_tests.rs"]
mod tests;
