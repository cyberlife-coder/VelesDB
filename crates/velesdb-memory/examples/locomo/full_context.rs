//! Full-context baseline: feed the ENTIRE conversation to the generator — no
//! retrieval, no memory system, no budget — and score the answers exactly as
//! the retrieval eval does. This is the reporting-standard local ceiling: the
//! accuracy a long-context model reaches when nothing is dropped, and the
//! token cost our budgeted retrieval trades that ceiling against.
//!
//! The whole conversation overflows Ollama's small default context window, so
//! generation pins `num_ctx` (see [`FULL_CTX_TOKENS`]); a silent truncation
//! there would quietly cap the very ceiling this baseline exists to measure.

use std::error::Error;

use crate::dataset::{Category, Sample};
use crate::judge;
use crate::ollama_gen::Generator;
use crate::report::pct;

/// Ollama context window for the whole-conversation prompt. Measured on the 10
/// `LoCoMo` conversations: the dated `- [YYYY-MM-DD] Speaker: text` lines run
/// ~2.9 chars/token (denser than a naive chars/4 estimate), so the prompt
/// averages ~29k tokens and the largest conversation reaches the low-30k range;
/// 49152 seats every one with comfortable headroom (verified: no prompt fills
/// the window), far under the model's 262144 limit. `run` refuses upfront (see
/// [`estimated_tokens`]) any conversation that would not fit.
const FULL_CTX_TOKENS: u64 = 49_152;

/// Tokens held back from [`FULL_CTX_TOKENS`] for the question, prompt scaffolding
/// and the generated answer when checking a conversation fits — Ollama shares
/// `num_ctx` between prompt and response, so the context alone must leave room.
const RESPONSE_RESERVE_TOKENS: u64 = 1_024;

/// Which stages route to the stronger Claude CLI instead of the local model:
/// `gen` for answer generation, `judge` for the correctness verdict. Both
/// default to the local model, matching the retrieval eval's flags.
#[derive(Clone, Copy)]
pub struct ClaudeFlags {
    pub gen: bool,
    pub judge: bool,
}

/// One category's running tally: correct / total, plus the prompt-token sum
/// (and its sample count) so the report can show the mean context cost paid.
#[derive(Default)]
struct Cell {
    n: u32,
    correct: u32,
    prompt_tokens: u64,
    token_samples: u32,
}

impl Cell {
    fn add(&mut self, correct: bool, prompt_tokens: Option<u64>) {
        self.n += 1;
        self.correct += u32::from(correct);
        if let Some(tokens) = prompt_tokens {
            self.prompt_tokens += tokens;
            self.token_samples += 1;
        }
    }

    fn accuracy(&self) -> f64 {
        pct(self.correct, self.n)
    }

    fn mean_prompt_tokens(&self) -> Option<u64> {
        (self.token_samples > 0).then(|| self.prompt_tokens / u64::from(self.token_samples))
    }
}

/// Per-category accuracy report for the full-context baseline.
#[derive(Default)]
pub struct FullContextReport {
    cells: [Cell; Category::COUNT],
}

impl FullContextReport {
    /// Answer one question from the whole conversation and fold the verdict in.
    /// Adversarial items are graded by abstention (the model should decline);
    /// answerable items by the LLM judge, mirroring the retrieval eval so the
    /// two numbers sit on the same scale.
    fn record(
        &mut self,
        generator: &Generator,
        context: &str,
        qa: &crate::dataset::Qa,
        claude: ClaudeFlags,
    ) -> Result<(), Box<dyn Error>> {
        let (candidate, usage) = answer(generator, context, &qa.question, claude.gen)?;
        let correct = if qa.category.is_adversarial() {
            judge::abstained(&candidate)
        } else if let Some(gold) = judge::gold_answer(qa) {
            judge::judge_correct(generator, &qa.question, gold, &candidate, claude.judge)?
        } else {
            false
        };
        self.cells[qa.category.index()].add(correct, usage.map(|u| u.prompt));
        Ok(())
    }

    /// Print the per-category accuracy table plus the answerable headline and
    /// the mean prompt-token cost the baseline paid to reach it. The provenance
    /// line reflects the *actual* answerer: with `--claude-gen` the pinned local
    /// `num_ctx` never applies (the Claude CLI answers), so it is not claimed.
    fn print(&self, generator: &Generator, samples: usize, gen_claude: bool) {
        println!(
            "\nVelesDB-memory — LoCoMo full-context baseline (no retrieval, whole conversation)"
        );
        let provenance = if gen_claude {
            "generator: claude (CLI)".to_string()
        } else {
            format!(
                "generator: {}   ·   num_ctx={FULL_CTX_TOKENS}",
                generator.model()
            )
        };
        println!("{provenance}   ·   {samples} conversation(s)\n");
        println!("  category         n      acc    mean-prompt-tokens");
        let mut answerable = Cell::default();
        for category in Category::ALL {
            let cell = &self.cells[category.index()];
            if cell.n == 0 {
                continue;
            }
            println!(
                "  {:<12} {:>4}    {:>4.0}%    {}",
                category.label(),
                cell.n,
                cell.accuracy(),
                cell.mean_prompt_tokens()
                    .map_or_else(|| "       —".to_string(), |t| format!("{t:>8}")),
            );
            if !category.is_adversarial() {
                answerable.n += cell.n;
                answerable.correct += cell.correct;
                answerable.prompt_tokens += cell.prompt_tokens;
                answerable.token_samples += cell.token_samples;
            }
        }
        println!(
            "  {:<12} {:>4}    {:>4.0}%    {}",
            "answerable",
            answerable.n,
            answerable.accuracy(),
            answerable
                .mean_prompt_tokens()
                .map_or_else(|| "       —".to_string(), |t| format!("{t:>8}")),
        );
        println!(
            "  (acc = LLM-judge accuracy; adversarial acc = abstention rate, excluded from the\n   \
answerable total; mean-prompt-tokens = Ollama's reported context size per live call, '—' when\n   \
every answer in the row was served from cache, which carries no usage counters.)"
        );
    }
}

/// Build the full-context prompt and generate a concise answer. Uses the same
/// [`judge::plain_answer_prompt`] the retrieval eval uses, so the baseline and
/// the eval grade like-for-like — only the "facts" differ: here the entire
/// dated conversation, not a budgeted pool.
fn answer(
    generator: &Generator,
    context: &str,
    question: &str,
    claude: bool,
) -> Result<(String, Option<crate::ollama_gen::TokenUsage>), Box<dyn Error>> {
    let prompt = judge::plain_answer_prompt(context, question);
    if claude {
        Ok((generator.judge(&prompt)?, None))
    } else {
        generator.generate_full_ctx(&prompt, FULL_CTX_TOKENS)
    }
}

/// Render the whole conversation as dated, speaker-attributed lines, oldest
/// first — the "memory" the baseline answers from. Each line carries its
/// session date so temporal questions are answerable in principle.
fn conversation_context(sample: &Sample) -> String {
    let mut lines = Vec::new();
    for session in &sample.sessions {
        let date = judge::fmt_date(session.date_key());
        for turn in &session.turns {
            match &date {
                Some(date) => lines.push(format!("- [{date}] {}: {}", turn.speaker, turn.text)),
                None => lines.push(format!("- {}: {}", turn.speaker, turn.text)),
            }
        }
    }
    lines.join("\n")
}

/// Conservative token estimate for the fit check, calibrated for the
/// Latin-script `LoCoMo` corpus this harness targets: its dated lines measured
/// ~2.9 chars/token, so dividing by a smaller 2.5 deliberately over-counts,
/// biasing the guard toward refusing a borderline conversation rather than
/// letting Ollama silently truncate it. Deterministic (no model call), so it
/// holds on a fully-cached rerun that reports no usage. Note the char-based
/// ratio would *under*-count heavily non-Latin text (CJK/emoji tokenize to
/// more than one token per char); the guard is a safety net for this dataset,
/// not a general cross-script bound.
fn estimated_tokens(text: &str) -> u64 {
    u64::try_from(text.chars().count()).unwrap_or(u64::MAX) * 2 / 5
}

/// Run the full-context baseline over the first `take` conversations. `only`
/// restricts scoring to a single category (a cheap, targeted A/B run), matching
/// the `--only` flag the retrieval eval honors.
pub fn run(
    generator: &Generator,
    samples: &[Sample],
    take: usize,
    claude: ClaudeFlags,
    max_qa: usize,
    only: Option<Category>,
) -> Result<(), Box<dyn Error>> {
    let contexts: Vec<(&Sample, String)> = samples
        .iter()
        .take(take)
        .map(|sample| (sample, conversation_context(sample)))
        .collect();
    // Pre-flight, before ANY generation: a conversation whose prompt would
    // overflow the pinned window is silently truncated by Ollama, corrupting the
    // ceiling. Catch every oversized conversation up front — deterministically,
    // independent of the answer cache — so the run aborts before spending
    // minutes generating the ones ahead of it, never folding a clipped answer
    // into a table that still looks clean. Skipped under `--claude-gen`, where
    // the Claude CLI answers and the local `num_ctx` never applies.
    if !claude.gen {
        ensure_all_fit(&contexts)?;
    }
    let mut report = FullContextReport::default();
    for (position, (sample, context)) in contexts.iter().enumerate() {
        let qa_take = crate::qa_limit(max_qa, sample.qa.len());
        println!(
            "[{}/{take}] {} — {} turns, {} QA (full context)",
            position + 1,
            sample.sample_id,
            sample.turn_count(),
            qa_take,
        );
        for qa in sample.qa.iter().take(qa_take) {
            if only.is_some_and(|category| category != qa.category) {
                continue;
            }
            report.record(generator, context, qa, claude)?;
        }
    }
    report.print(generator, take, claude.gen);
    Ok(())
}

/// Verify every conversation's whole-context prompt fits the pinned window
/// before any generation runs, so an oversized conversation aborts the run up
/// front instead of after the earlier conversations were already (expensively)
/// generated. Deterministic, so it holds on a fully-cached rerun.
fn ensure_all_fit(contexts: &[(&Sample, String)]) -> Result<(), Box<dyn Error>> {
    for (sample, context) in contexts {
        let needed = estimated_tokens(context) + RESPONSE_RESERVE_TOKENS;
        if needed > FULL_CTX_TOKENS {
            return Err(format!(
                "conversation {} needs ~{needed} context tokens but num_ctx is \
{FULL_CTX_TOKENS}; raise FULL_CTX_TOKENS (up to 262144) — note this rebinds the \
generation cache key, so cached full-context answers will be regenerated",
                sample.sample_id,
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "full_context/full_context_tests.rs"]
mod tests;
