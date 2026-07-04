//! Dated context: turn recalled facts into a chronological, date-prefixed
//! timeline with a "now" anchor — the representation measured to lift temporal
//! question answering (the `examples/locomo` temporal ablation: +33.6pp,
//! McNemar p=1.8e-28). This ships that representation as product behavior so a
//! caller reproduces it through the installed API instead of re-implementing
//! the formatting in a prompt.
//!
//! The date lives in caller-supplied metadata under a field the caller names
//! (e.g. `ts`, `occurred_at`), holding a `YYYYMMDD` integer — the same key
//! shape `recall_where` filters on. A fact whose named field is missing or not
//! a valid `YYYYMMDD` date is treated as undated: it still appears, just without
//! a date prefix and after the dated timeline, so an unlabeled fact never
//! invents a misleading date.
//!
//! Only the *formatting* ships here — not the LLM reasoning prompt the harness
//! wraps around it. Presenting retrieved facts is a memory-store concern;
//! prompt engineering is the caller's.

use crate::model::Recollection;

/// A chronological, date-prefixed rendering of recalled facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatedContext {
    /// One line per fact: `- [YYYY-MM-DD] content` for a dated fact, `- content`
    /// for an undated one. Dated facts come first in ascending date order; any
    /// undated facts follow in their original (relevance) order.
    pub timeline: String,
    /// The most recent date across the facts (`YYYY-MM-DD`), the natural "now"
    /// anchor for temporal reasoning. `None` when no fact carries a valid date.
    pub now: Option<String>,
}

/// Render `facts` as a [`DatedContext`], reading each fact's date from the
/// `date_field` metadata key (a `YYYYMMDD` integer).
///
/// Facts are split into dated (sorted oldest-first) and undated (kept in the
/// order given, i.e. by relevance), then rendered one per line. `now` is the
/// latest date seen. An empty `facts` yields an empty timeline and `now: None`.
#[must_use]
pub fn format_dated_context(facts: &[Recollection], date_field: &str) -> DatedContext {
    // (date, content) for dated facts; content only for undated ones.
    let mut dated: Vec<(i64, &str)> = Vec::new();
    let mut undated: Vec<&str> = Vec::new();
    for fact in facts {
        match fact_date(fact, date_field) {
            Some(date) => dated.push((date, &fact.content)),
            None => undated.push(&fact.content),
        }
    }
    // Ascending chronological order; a stable sort keeps same-date facts in
    // their original relevance order.
    dated.sort_by_key(|(date, _)| *date);
    let now = dated.last().and_then(|(date, _)| fmt_date(*date));

    let lines = dated
        .iter()
        .map(|(date, content)| match fmt_date(*date) {
            Some(date) => format!("- [{date}] {content}"),
            // Unreachable in practice: a value is only in `dated` because
            // `fact_date` (via `decompose_ymd`) already validated it. Kept total
            // rather than `unwrap` so a future change can't panic here.
            None => format!("- {content}"),
        })
        .chain(undated.iter().map(|content| format!("- {content}")))
        .collect::<Vec<_>>()
        .join("\n");

    DatedContext {
        timeline: lines,
        now,
    }
}

/// A fact's date from its `date_field` metadata, or `None` when the field is
/// absent, non-integer, or not a valid `YYYYMMDD`.
fn fact_date(fact: &Recollection, date_field: &str) -> Option<i64> {
    let raw = fact.metadata.as_ref()?.get(date_field)?.as_i64()?;
    // Validate the calendar shape so an out-of-range integer (e.g. a plain
    // counter that happens to live under the date field) is treated as undated,
    // not rendered as a nonsense date.
    decompose_ymd(raw).map(|_| raw)
}

/// Render a `YYYYMMDD` integer as `YYYY-MM-DD`, or `None` when it is not a valid
/// calendar date (`<= 0`, or month/day out of range).
fn fmt_date(ts: i64) -> Option<String> {
    let (year, month, day) = decompose_ymd(ts)?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Split a `YYYYMMDD` integer into `(year, month, day)`, or `None` when it is
/// `<= 0` or the month/day is out of range. The single validity rule for the
/// date convention, mirroring the `examples/locomo` harness.
fn decompose_ymd(ts: i64) -> Option<(i64, i64, i64)> {
    if ts <= 0 {
        return None;
    }
    let (year, month, day) = (ts / 10_000, (ts / 100) % 100, ts % 100);
    ((1..=12).contains(&month) && (1..=31).contains(&day)).then_some((year, month, day))
}

#[cfg(test)]
#[path = "dated_context_tests.rs"]
mod tests;
