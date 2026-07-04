//! Unit tests for the dated-context formatter.

use super::*;
use crate::model::Recollection;
use serde_json::json;

/// A recollection with `content` and an optional `date_field` metadata date.
fn fact(id: u64, content: &str, date_field: &str, date: Option<i64>) -> Recollection {
    let metadata = date.map(|d| {
        let mut m = serde_json::Map::new();
        m.insert(date_field.to_string(), json!(d));
        m
    });
    Recollection {
        id,
        score: 0.0,
        content: content.to_string(),
        metadata,
    }
}

#[test]
fn empty_facts_yield_empty_timeline_and_no_now() {
    let ctx = format_dated_context(&[], "ts");
    assert_eq!(ctx.timeline, "");
    assert_eq!(ctx.now, None);
}

#[test]
fn dated_facts_are_sorted_ascending_and_prefixed() {
    // Given out of order; must render oldest-first with date prefixes.
    let facts = vec![
        fact(1, "release shipped", "ts", Some(20_260_701)),
        fact(2, "kickoff meeting", "ts", Some(20_260_103)),
    ];
    let ctx = format_dated_context(&facts, "ts");
    assert_eq!(
        ctx.timeline,
        "- [2026-01-03] kickoff meeting\n- [2026-07-01] release shipped"
    );
    // "now" anchors on the latest date.
    assert_eq!(ctx.now.as_deref(), Some("2026-07-01"));
}

#[test]
fn undated_facts_follow_the_timeline_without_a_prefix() {
    let facts = vec![
        fact(1, "dated one", "ts", Some(20_260_101)),
        fact(2, "no date here", "ts", None),
    ];
    let ctx = format_dated_context(&facts, "ts");
    assert_eq!(ctx.timeline, "- [2026-01-01] dated one\n- no date here");
    assert_eq!(ctx.now.as_deref(), Some("2026-01-01"));
}

#[test]
fn all_undated_facts_have_no_now_anchor() {
    let facts = vec![fact(1, "a", "ts", None), fact(2, "b", "ts", None)];
    let ctx = format_dated_context(&facts, "ts");
    assert_eq!(ctx.timeline, "- a\n- b");
    assert_eq!(ctx.now, None);
}

#[test]
fn the_date_field_name_is_honored() {
    // Same fact, a caller-chosen field name other than `ts`.
    let facts = vec![fact(1, "hired", "occurred_at", Some(20_250_615))];
    let ctx = format_dated_context(&facts, "occurred_at");
    assert_eq!(ctx.timeline, "- [2025-06-15] hired");
    // A different field name sees no date → treated as undated.
    let ctx_wrong = format_dated_context(&facts, "ts");
    assert_eq!(ctx_wrong.timeline, "- hired");
    assert_eq!(ctx_wrong.now, None);
}

#[test]
fn out_of_range_or_non_integer_dates_are_treated_as_undated() {
    // 20261301 = month 13; 20260231 = Feb 31 (impossible day for the month);
    // 20260229 = Feb 29 in a non-leap year; 0 and negatives rejected; a string
    // is not an integer date. None must render a date prefix or a `now` anchor.
    let bad_month = fact(1, "bad month", "ts", Some(20_261_301));
    let feb31 = fact(2, "feb 31", "ts", Some(20_260_231));
    let feb29_non_leap = fact(3, "feb 29 non-leap", "ts", Some(20_260_229));
    let zero = fact(4, "zero", "ts", Some(0));
    let mut string_date = fact(5, "string date", "ts", None);
    let mut m = serde_json::Map::new();
    m.insert("ts".to_string(), json!("2026-07-01"));
    string_date.metadata = Some(m);

    let ctx = format_dated_context(&[bad_month, feb31, feb29_non_leap, zero, string_date], "ts");
    assert_eq!(
        ctx.timeline,
        "- bad month\n- feb 31\n- feb 29 non-leap\n- zero\n- string date"
    );
    assert_eq!(ctx.now, None);
}

#[test]
fn valid_end_of_month_and_leap_day_dates_are_kept() {
    // Real edge dates must still render: Feb 29 in a leap year, 30/31-day ends.
    let facts = vec![
        fact(1, "leap day", "ts", Some(20_240_229)),
        fact(2, "april end", "ts", Some(20_260_430)),
        fact(3, "december end", "ts", Some(20_261_231)),
    ];
    let ctx = format_dated_context(&facts, "ts");
    assert_eq!(
        ctx.timeline,
        "- [2024-02-29] leap day\n- [2026-04-30] april end\n- [2026-12-31] december end"
    );
    assert_eq!(ctx.now.as_deref(), Some("2026-12-31"));
}

#[test]
fn same_date_facts_keep_relevance_order() {
    // A stable sort must not reorder facts sharing a date.
    let facts = vec![
        fact(1, "first by relevance", "ts", Some(20_260_101)),
        fact(2, "second by relevance", "ts", Some(20_260_101)),
    ];
    let ctx = format_dated_context(&facts, "ts");
    assert_eq!(
        ctx.timeline,
        "- [2026-01-01] first by relevance\n- [2026-01-01] second by relevance"
    );
}
