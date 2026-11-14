//! Line-level pruning of one tool result under a token budget — the
//! `compile-stdin --tool-output` path the agent `PostToolUse` hook calls.
//!
//! # Why tool output is not compiled like a transcript
//!
//! The fragment compiler decides per *fragment*, and its critical rules are
//! existence tests: `preserve.exact_values` fires on three tokens carrying a
//! digit, `preserve.negative_constraint` on one "never". A shell command's
//! output is one fragment, or a few kilobyte-sized ones, and nearly every
//! line of an `ls -l`, a `git log --stat` or a changelog carries digits — so
//! every fragment was critical, and any budget at all reported
//! `risk: high`, which the hook refuses. Measured on those three outputs at a
//! 4 000-token budget: `high` whole, and still `high` cut into 2 KiB
//! fragments.
//!
//! Recent work on agent observations points the same way: decide per *line*,
//! conditioned on the task, and keep the result reversible — Squeez
//! (arXiv:2604.04979) and SWE-Pruner Pro (arXiv:2607.18213) prune line by
//! line against the agent's query; DTOC (arXiv:2609.26121) attributes its
//! accuracy to keeping the full output retrievable behind a placeholder.
//! This module is the deterministic version of that: no model, no regex, no
//! clock.
//!
//! # What is kept
//!
//! 1. Consecutive identical lines collapse to one, annotated `[×N]`.
//! 2. **Diagnostic** lines are kept verbatim first: lines shaped like a
//!    tool's report of a problem (`error:`, `error[E…]`, `FAILED`,
//!    `panicked at`, `Traceback (most recent call last)`, `exit code`, …).
//!    The shape matters, not the word: a changelog sentence about a fixed
//!    error is prose, and treating every such word as critical is how the
//!    first version of this pruner still reported `high` on `cat CHANGELOG.md`.
//! 3. Then negative constraints (the line-level test
//!    `preserve.negative_constraint` uses): the line an agent must not act
//!    against.
//! 4. Then the last [`ToolOutputPolicy::tail_lines`] (summaries print last)
//!    and the first [`ToolOutputPolicy::head_lines`].
//! 5. Then lines naming a term of the caller's query — its non-flag
//!    arguments, since the program name and flags echo everywhere in their
//!    own output — and [`ToolOutputPolicy::context_lines`] around each
//!    diagnostic line.
//! 6. Remaining budget goes to the lines nearest what is already kept.
//!
//! An output whose whole-text estimate already fits is returned unpruned:
//! the per-line costs the selection uses over-approximate on short lines.
//!
//! Every elided run is replaced in place by `[… N lines elided …]`, so the
//! reader knows exactly where and how much is missing.
//!
//! # Fidelity risk
//!
//! [`FidelityRisk::Low`] when nothing was elided; [`FidelityRisk::Medium`]
//! when only non-diagnostic lines were — the caller holds the original (the
//! hook archives it and names the path in its footer); [`FidelityRisk::High`]
//! when a diagnostic line did not fit, which keeps the hook's refusal for the
//! case it exists for: a view of a failing run with its failure cut out.

use std::collections::VecDeque;

use super::classify::line_has_negative_constraint;
use super::estimator::TokenEstimator;
use super::model::FidelityRisk;

/// Lowercased substrings that mark a tool's own report of a problem.
const DIAGNOSTIC_MARKERS: &[&str] = &[
    "error:",
    "error[",
    "warning:",
    "warning[",
    "fatal:",
    "failed:",
    "failures:",
    "panicked at",
    "traceback (most recent call last)",
    "exception:",
    "assertionerror",
    "test result: failed",
    "segmentation fault",
    "exit code",
    "exit status",
    "command not found",
    "no such file or directory",
    "permission denied",
    "timed out",
];

/// Whole tokens that mark a problem only in upper case (`FAILED`, not
/// "failed to reproduce" in prose).
const UPPERCASE_MARKERS: &[&str] = &[
    "ERROR", "FAILED", "FAIL", "FATAL", "PANIC", "CRITICAL", "WARN", "WARNING",
];

/// Query terms shorter than this match too much to mean anything.
const MIN_QUERY_TERM_CHARS: usize = 3;

/// How many framing lines [`prune_tool_output`] keeps around the diagnostic ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutputPolicy {
    /// Leading lines kept: what the command printed first. Default `5`.
    pub head_lines: usize,
    /// Trailing lines kept, before the head: summaries and exit status print
    /// last. Default `10`.
    pub tail_lines: usize,
    /// Lines kept on each side of a diagnostic line. Default `1`.
    pub context_lines: usize,
}

impl Default for ToolOutputPolicy {
    fn default() -> Self {
        Self {
            head_lines: 5,
            tail_lines: 10,
            context_lines: 1,
        }
    }
}

/// The result of [`prune_tool_output`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrunedToolOutput {
    /// Kept lines in their original order, each elided run replaced by one
    /// `[… N lines elided …]` marker.
    pub content: String,
    /// Estimated tokens of the input.
    pub tokens_in: u64,
    /// Estimated tokens of [`Self::content`].
    pub tokens_out: u64,
    /// See the module docs' "Fidelity risk".
    pub risk: FidelityRisk,
    /// Lines (after duplicate collapsing) replaced by markers.
    pub elided_lines: usize,
    /// How many of those were diagnostic lines — the losses that make the
    /// risk [`FidelityRisk::High`].
    pub diagnostic_dropped: usize,
}

/// Prune `text` to at most `token_budget` estimated tokens — see the module
/// docs. Pure and deterministic: the same inputs always give the same output.
#[must_use]
pub fn prune_tool_output(
    text: &str,
    query: &str,
    token_budget: u64,
    policy: &ToolOutputPolicy,
    estimator: &dyn TokenEstimator,
) -> PrunedToolOutput {
    let lines = collapse_consecutive(text);
    // Per-line costs below over-approximate (each line rounds up and pays for
    // its newline), so an output that fits as a whole is never pruned by them.
    let whole = lines.join("\n");
    if estimator.estimate(&whole) <= token_budget {
        return PrunedToolOutput {
            tokens_in: estimator.estimate(text),
            tokens_out: estimator.estimate(&whole),
            risk: FidelityRisk::Low,
            content: whole,
            elided_lines: 0,
            diagnostic_dropped: 0,
        };
    }
    let classes = LineClasses::of(&lines, &query_terms(query));
    let diagnostic = &classes.diagnostic;
    let costs: Vec<u64> = lines
        .iter()
        .map(|line| estimator.estimate(line) + 1)
        .collect();
    let marker_cost = estimator.estimate("[… 99999 lines elided …]") + 1;
    let mut selection = Selection::new(&costs, marker_cost, token_budget);
    for index in priority_order(&classes, policy) {
        selection.try_keep(index);
    }
    let content = render(&lines, &selection.kept);
    let elided_lines = selection.kept.iter().filter(|kept| !**kept).count();
    let diagnostic_dropped = (0..lines.len())
        .filter(|&i| diagnostic[i] && !selection.kept[i])
        .count();
    PrunedToolOutput {
        tokens_in: estimator.estimate(text),
        tokens_out: estimator.estimate(&content),
        risk: risk_of(elided_lines, diagnostic_dropped),
        content,
        elided_lines,
        diagnostic_dropped,
    }
}

fn risk_of(elided_lines: usize, diagnostic_dropped: usize) -> FidelityRisk {
    if diagnostic_dropped > 0 {
        FidelityRisk::High
    } else if elided_lines > 0 {
        FidelityRisk::Medium
    } else {
        FidelityRisk::Low
    }
}

/// Lines with each run of identical consecutive lines folded into its first,
/// annotated with the run length.
fn collapse_consecutive(text: &str) -> Vec<String> {
    let mut lines: Vec<(String, usize)> = Vec::new();
    for line in text.lines() {
        match lines.last_mut() {
            Some((last, count)) if last == line => *count += 1,
            _ => lines.push((line.to_owned(), 1)),
        }
    }
    lines
        .into_iter()
        .map(|(line, count)| {
            if count > 1 {
                format!("{line} [×{count}]")
            } else {
                line
            }
        })
        .collect()
}

/// The query's specific words: its arguments, lowercased — not the first
/// word (the program) nor flags, which echo throughout their own output.
fn query_terms(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .skip(1)
        .filter(|word| !word.starts_with('-'))
        .flat_map(|word| word.split(|c: char| !c.is_alphanumeric() && c != '_'))
        .filter(|term| term.chars().count() >= MIN_QUERY_TERM_CHARS)
        .map(str::to_lowercase)
        .collect()
}

fn is_diagnostic(line: &str) -> bool {
    let lowered = line.to_lowercase();
    DIAGNOSTIC_MARKERS
        .iter()
        .any(|marker| lowered.contains(marker))
        || line
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|token| UPPERCASE_MARKERS.contains(&token))
}

fn names_a_query_term(line: &str, terms: &[String]) -> bool {
    let lowered = line.to_lowercase();
    terms.iter().any(|term| lowered.contains(term.as_str()))
}

/// What each line is, as far as the budget order cares.
struct LineClasses {
    diagnostic: Vec<bool>,
    constraint: Vec<bool>,
    query: Vec<bool>,
}

impl LineClasses {
    fn of(lines: &[String], terms: &[String]) -> Self {
        Self {
            diagnostic: lines.iter().map(|line| is_diagnostic(line)).collect(),
            constraint: lines
                .iter()
                .map(|line| line_has_negative_constraint(line))
                .collect(),
            query: lines
                .iter()
                .map(|line| names_a_query_term(line, terms))
                .collect(),
        }
    }
}

/// Every line index once, in the order the budget should be spent:
/// diagnostic, negative constraint, tail, head, query term, diagnostic
/// context, then the rest by distance to those. A constraint outranks the
/// framing because it is the line an agent must not act against (`do NOT run
/// cargo clean` in a build log); its loss still reads `medium`, not `high`,
/// because changelog prose is full of "never".
fn priority_order(classes: &LineClasses, policy: &ToolOutputPolicy) -> Vec<usize> {
    let len = classes.diagnostic.len();
    let flagged = |flags: &[bool]| (0..len).filter(|&i| flags[i]).collect::<Vec<_>>();
    let mut order = flagged(&classes.diagnostic);
    order.extend(flagged(&classes.constraint));
    order.extend(len.saturating_sub(policy.tail_lines)..len);
    order.extend(0..policy.head_lines.min(len));
    order.extend(flagged(&classes.query));
    for index in flagged(&classes.diagnostic) {
        let start = index.saturating_sub(policy.context_lines);
        let end = (index + policy.context_lines + 1).min(len);
        order.extend(start..end);
    }
    let anchored = dedup_preserving_order(order, len);
    let distance = distances_from(&anchored, len);
    let mut rest: Vec<usize> = (0..len).filter(|&i| distance[i] > 0).collect();
    rest.sort_by_key(|&i| (distance[i], i));
    anchored.into_iter().chain(rest).collect()
}

fn dedup_preserving_order(order: Vec<usize>, len: usize) -> Vec<usize> {
    let mut seen = vec![false; len];
    order
        .into_iter()
        .filter(|&i| !std::mem::replace(&mut seen[i], true))
        .collect()
}

/// Line distance to the nearest anchored line (0 for the anchors); with no
/// anchor, the distance to the top, so the rest fills in reading order.
fn distances_from(anchors: &[usize], len: usize) -> Vec<usize> {
    if anchors.is_empty() {
        return (1..=len).collect();
    }
    let mut distance = vec![usize::MAX; len];
    let mut queue: VecDeque<usize> = anchors.iter().copied().collect();
    for &anchor in anchors {
        distance[anchor] = 0;
    }
    while let Some(index) = queue.pop_front() {
        let next = distance[index] + 1;
        for neighbour in [index.checked_sub(1), Some(index + 1).filter(|&n| n < len)]
            .into_iter()
            .flatten()
        {
            if distance[neighbour] == usize::MAX {
                distance[neighbour] = next;
                queue.push_back(neighbour);
            }
        }
    }
    distance
}

/// Which lines are kept, and what the rendering costs so far: kept lines
/// plus one marker per elided run. Starts with everything elided.
struct Selection<'a> {
    costs: &'a [u64],
    marker_cost: u64,
    budget: u64,
    kept: Vec<bool>,
    spent: u64,
}

impl<'a> Selection<'a> {
    fn new(costs: &'a [u64], marker_cost: u64, budget: u64) -> Self {
        let runs = u64::from(!costs.is_empty());
        Self {
            costs,
            marker_cost,
            budget,
            kept: vec![false; costs.len()],
            spent: runs * marker_cost,
        }
    }

    /// Keep line `index` when the rendering still fits the budget after it.
    /// Keeping a line splits, shortens or closes the elided run around it:
    /// one marker more, the same, or one fewer.
    fn try_keep(&mut self, index: usize) {
        let left_kept = index == 0 || self.kept[index - 1];
        let right_kept = index + 1 == self.kept.len() || self.kept[index + 1];
        let after = match (left_kept, right_kept) {
            (true, true) => (self.spent + self.costs[index]).saturating_sub(self.marker_cost),
            (false, false) => self.spent + self.costs[index] + self.marker_cost,
            _ => self.spent + self.costs[index],
        };
        if after <= self.budget {
            self.kept[index] = true;
            self.spent = after;
        }
    }
}

fn render(lines: &[String], kept: &[bool]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut elided = 0_usize;
    for (line, &keep) in lines.iter().zip(kept) {
        if keep {
            flush_marker(&mut out, &mut elided);
            out.push(line.clone());
        } else {
            elided += 1;
        }
    }
    flush_marker(&mut out, &mut elided);
    out.join("\n")
}

fn flush_marker(out: &mut Vec<String>, elided: &mut usize) {
    if *elided > 0 {
        let noun = if *elided == 1 { "line" } else { "lines" };
        out.push(format!("[… {elided} {noun} elided …]"));
        *elided = 0;
    }
}

#[cfg(test)]
#[path = "tool_output_tests.rs"]
mod tests;
