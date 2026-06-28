//! Answer generation and hybrid scoring.
//!
//! Hybrid, as chosen for this benchmark: a local LLM judge gives a citable
//! correctness verdict, and a deterministic token-F1 is logged beside it as a
//! reproducibility guard. Adversarial (unanswerable) items are scored by
//! abstention — the model should say it cannot answer — and skip both the LLM
//! judge and F1.

use std::error::Error;

use crate::dataset::Qa;
use crate::ollama_gen::Generator;
use crate::parse::{normalize, tokens};

const ABSTAIN: &str = "NO_ANSWER";

/// Generate a concise answer from the retrieved facts (each a `(YYYYMMDD ts,
/// text)` pair), or the abstain token when the facts don't contain the answer.
/// When `date_context` is set, every dated fact is prefixed with its date and
/// the facts are ordered chronologically — the session date is retrieved but
/// otherwise invisible to the answerer, which makes temporal questions
/// unanswerable despite high evidence recall.
// benchmark harness: ablation knobs threaded through the harness
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub fn answer(
    generator: &Generator,
    question: &str,
    facts: &[(i64, String)],
    date_context: bool,
    now_ts: i64,
    scaffold: bool,
    cot: bool,
    claude_gen: bool,
) -> Result<String, Box<dyn Error>> {
    let context = if facts.is_empty() {
        "(no facts retrieved)".to_string()
    } else {
        let mut ordered: Vec<&(i64, String)> = facts.iter().collect();
        if date_context {
            // Chronological so interval/ordering questions read naturally.
            ordered.sort_by_key(|(ts, _)| *ts);
        }
        ordered
            .iter()
            .map(
                |(ts, text)| match date_context.then(|| fmt_date(*ts)).flatten() {
                    Some(date) => format!("- [{date}] {text}"),
                    None => format!("- {text}"),
                },
            )
            .collect::<Vec<_>>()
            .join("\n")
    };
    if scaffold {
        // Temporal reasoning scaffold: a dated, chronological timeline + a "now"
        // anchor + explicit date-arithmetic, ending in a parseable FINAL line.
        let now = fmt_date(now_ts).unwrap_or_else(|| "unknown".to_string());
        let prompt = format!(
            "You answer a temporal question from a dated memory timeline (each line \
is '- [YYYY-MM-DD] fact', in chronological order). Today's date is {now}.\n\n\
Timeline:\n{context}\n\nQuestion: {question}\n\n\
Reason step by step: pick the relevant dated fact(s), then compute the interval, \
ordering, or date the question asks for. If the timeline does not contain the \
answer, the final answer is {ABSTAIN}. End with a line exactly of the form:\n\
FINAL: <answer in a few words>"
        );
        let raw = gen(generator, &prompt, claude_gen)?;
        return Ok(extract_final(&raw));
    }
    if cot {
        // General chain-of-thought: reason over the facts, then a parseable FINAL.
        let prompt = format!(
            "Answer the question using ONLY the memory facts below.\n\n\
Facts:\n{context}\n\nQuestion: {question}\n\n\
Think step by step: identify the relevant fact(s) and how they connect to the \
question, then determine the answer. If the facts do not contain the answer, the \
final answer is {ABSTAIN}. End with a line exactly of the form:\n\
FINAL: <answer in a few words>"
        );
        let raw = gen(generator, &prompt, claude_gen)?;
        return Ok(extract_final(&raw));
    }
    let prompt = format!(
        "Answer the question using ONLY the memory facts below. If the facts do \
not contain the answer, reply exactly {ABSTAIN}. Be concise: a few words, no \
explanation.\n\nFacts:\n{context}\n\nQuestion: {question}\nAnswer:"
    );
    gen(generator, &prompt, claude_gen)
}

/// Generate an answer with either the strong external model (Claude via CLI) or
/// the local model, depending on `claude_gen`.
fn gen(generator: &Generator, prompt: &str, claude_gen: bool) -> Result<String, Box<dyn Error>> {
    if claude_gen {
        generator.judge(prompt)
    } else {
        generator.generate(prompt)
    }
}

/// Extract the answer from a scaffolded reply: the text after the last `FINAL:`
/// line (case-insensitive), or the whole reply if no such line is present.
fn extract_final(text: &str) -> String {
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("final:") {
            return trimmed[6..].trim().to_string();
        }
    }
    text.trim().to_string()
}

/// Render a `YYYYMMDD` session key as `YYYY-MM-DD`, or `None` when unknown (0)
/// or malformed, so an undated fact is shown without a misleading date prefix.
fn fmt_date(ts: i64) -> Option<String> {
    if ts <= 0 {
        return None;
    }
    let (year, month, day) = (ts / 10_000, (ts / 100) % 100, ts % 100);
    if (1..=12).contains(&month) && (1..=31).contains(&day) {
        Some(format!("{year:04}-{month:02}-{day:02}"))
    } else {
        None
    }
}

/// True when the model declined to answer (the correct move on adversarial QA).
/// Matched on the leading token, not a substring, so a real answer that merely
/// contains the words "no answer" is not mistaken for an abstention.
pub fn abstained(model_answer: &str) -> bool {
    let norm = normalize(model_answer);
    if norm.is_empty() || norm == "no answer" {
        return true;
    }
    let head: String = norm
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    head == ABSTAIN.to_lowercase()
}

/// LLM-judge verdict for an answerable question: does the candidate convey the
/// reference answer's information? Returns `false` on an abstention.
pub fn judge_correct(
    generator: &Generator,
    question: &str,
    gold: &str,
    candidate: &str,
    use_claude: bool,
) -> Result<bool, Box<dyn Error>> {
    if abstained(candidate) {
        return Ok(false);
    }
    let prompt = format!(
        "You are grading a question-answering system. Decide if the candidate \
answer conveys the same information as the reference answer (a superset is \
fine; extra correct detail is fine). Reply with exactly one word: CORRECT or \
INCORRECT.\n\nQuestion: {question}\nReference answer: {gold}\nCandidate answer: \
{candidate}\nVerdict:"
    );
    // The judge can run on the stronger Claude model (vendor-neutral, fairer) or
    // the local model; the candidate answer itself is always the local system.
    let raw = if use_claude {
        generator.judge(&prompt)?
    } else {
        generator.generate(&prompt)?
    };
    let verdict = normalize(&raw);
    // Compare the leading word so "incorrect" never matches "correct" by
    // substring, and a stray verbose reply still grades on its first verdict word.
    let head: String = verdict
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    Ok(head == "correct")
}

/// Token-level F1 (SQuAD-style multiset overlap) between candidate and gold;
/// 0.0 for an abstention.
pub fn f1(candidate: &str, gold: &str) -> f64 {
    if abstained(candidate) {
        return 0.0;
    }
    let cand = tokens(candidate);
    let gold = tokens(gold);
    if cand.is_empty() || gold.is_empty() {
        return 0.0;
    }
    let overlap = multiset_overlap(&cand, &gold);
    if overlap == 0 {
        return 0.0;
    }
    let precision = ratio(overlap, cand.len());
    let recall = ratio(overlap, gold.len());
    2.0 * precision * recall / (precision + recall)
}

/// `num / den` as `f64`, widening through `u32` (token counts never approach
/// its range; the `unwrap_or` clamp is a belt-and-braces guard, not expected).
fn ratio(num: usize, den: usize) -> f64 {
    let num = u32::try_from(num).unwrap_or(u32::MAX);
    let den = u32::try_from(den).unwrap_or(u32::MAX);
    if den == 0 {
        return 0.0;
    }
    f64::from(num) / f64::from(den)
}

/// Sum over shared tokens of `min(count_in_a, count_in_b)` — true multiset
/// intersection size, so repeated words count only as often as both contain.
fn multiset_overlap(a: &[String], b: &[String]) -> usize {
    let mut counts: std::collections::HashMap<&str, i32> = std::collections::HashMap::new();
    for token in a {
        *counts.entry(token).or_insert(0) += 1;
    }
    let mut overlap = 0usize;
    for token in b {
        let slot = counts.entry(token).or_insert(0);
        if *slot > 0 {
            *slot -= 1;
            overlap += 1;
        }
    }
    overlap
}

/// Reference answer text for scoring, if the item is answerable.
pub fn gold_answer(qa: &Qa) -> Option<&str> {
    qa.answer.as_deref()
}
