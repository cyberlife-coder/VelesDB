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
    // (sort key, pre-formatted `YYYY-MM-DD`, content) for dated facts; content
    // only for undated ones. The date is formatted once here, so nothing
    // downstream re-parses it.
    let mut dated: Vec<(i64, String, &str)> = Vec::new();
    let mut undated: Vec<&str> = Vec::new();
    for fact in facts {
        match fact_date(fact, date_field) {
            Some((key, formatted)) => dated.push((key, formatted, &fact.content)),
            None => undated.push(&fact.content),
        }
    }
    // Ascending chronological order; a stable sort keeps same-date facts in
    // their original relevance order.
    dated.sort_by_key(|(key, _, _)| *key);
    let now = dated.last().map(|(_, formatted, _)| formatted.clone());

    let lines = dated
        .iter()
        .map(|(_, date, content)| format!("- [{date}] {content}"))
        .chain(undated.iter().map(|content| format!("- {content}")))
        .collect::<Vec<_>>()
        .join("\n");

    DatedContext {
        timeline: lines,
        now,
    }
}

/// A fact's `(sort key, formatted "YYYY-MM-DD")` from its `date_field`
/// metadata, or `None` when the field is absent, non-integer, or not a valid
/// calendar date — so a plain counter (or an impossible date like `20260231`)
/// living under the date field is treated as undated, not rendered as a
/// nonsense timeline anchor. Formats the date here so the caller never parses
/// the integer twice.
fn fact_date(fact: &Recollection, date_field: &str) -> Option<(i64, String)> {
    let raw = fact.metadata.as_ref()?.get(date_field)?.as_i64()?;
    Some((raw, fmt_date(raw)?))
}

/// Render a `YYYYMMDD` integer as `YYYY-MM-DD`, or `None` when it is not a valid
/// calendar date.
fn fmt_date(ts: i64) -> Option<String> {
    let (year, month, day) = decompose_ymd(ts)?;
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Split a `YYYYMMDD` integer into `(year, month, day)`, or `None` when it is
/// `<= 0`, the month is out of range, or the day exceeds that month's real
/// length (leap years included) — the single validity rule for the date
/// convention, stricter than the harness's `1..=31` so no impossible date ever
/// reaches the timeline.
fn decompose_ymd(ts: i64) -> Option<(i64, i64, i64)> {
    if ts <= 0 {
        return None;
    }
    let (year, month, day) = (ts / 10_000, (ts / 100) % 100, ts % 100);
    if !(1..=12).contains(&month) {
        return None;
    }
    (1..=days_in_month(year, month))
        .contains(&day)
        .then_some((year, month, day))
}

/// Days in `month` (1..=12) of `year` in the proleptic Gregorian calendar
/// (February is 29 in a leap year). Only ever called with a validated month.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Whether `year` is a leap year (Gregorian rule).
fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

#[cfg(test)]
#[path = "dated_context_tests.rs"]
mod tests;
