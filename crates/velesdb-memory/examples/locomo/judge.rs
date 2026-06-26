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

/// Generate a concise answer from the retrieved fact texts, or the abstain
/// token when the facts don't contain the answer.
pub fn answer(
    generator: &Generator,
    question: &str,
    facts: &[String],
) -> Result<String, Box<dyn Error>> {
    let context = if facts.is_empty() {
        "(no facts retrieved)".to_string()
    } else {
        facts
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let prompt = format!(
        "Answer the question using ONLY the memory facts below. If the facts do \
not contain the answer, reply exactly {ABSTAIN}. Be concise: a few words, no \
explanation.\n\nFacts:\n{context}\n\nQuestion: {question}\nAnswer:"
    );
    generator.generate(&prompt)
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
    let verdict = normalize(&generator.generate(&prompt)?);
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
